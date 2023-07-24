/* video.rs
 *
 * Copyright 2021-2022 Bohong Huang
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program. If not, see <http://www.gnu.org/licenses/>.
 */

use std::{str::FromStr, sync::{Arc, Mutex}, ffi::c_void};

use glib::{Sender, clone, EnumClass};
use gtk::prelude::*;
use gst::{Element, Pad, PadProbeType, Pipeline, element_error, 
            prelude::*, PadProbeReturn, PadProbeData, EventView};
use gdk_pixbuf::{Colorspace, Pixbuf};

use opencv as cv;
use cv::{core::VecN, types::VectorOfMat};
use cv::{prelude::*, Result, imgproc, core::Size};

use strum_macros::{EnumIter, Display as EnumToString};
use url::Url;

use crate::async_glib::{Future, Promise};

use super::slave_config::SlaveConfigModel;

#[derive(EnumIter, EnumToString, PartialEq, Clone, Debug)]
pub enum VideoAlgorithm {
    CLAHE
}

pub fn connect_elements_to_pipeline(pipeline: &Pipeline, tee_name: &str, elements: &[Element]) -> Result<(Element, Pad), String> {
    let output_tee = pipeline.by_name(tee_name).ok_or("Cannot find output_tee")?; // 通过名称获取输出分支

    if let Some(element) = elements.first() {
        pipeline.add(element).map_err(|_| "Cannot add the first element to pipeline")?; // 必须先添加，再连接第一个元素到管中
    }

    let teepad = output_tee.request_pad_simple("src_%u").ok_or("Cannot request pad")?; // 请求输出分支的端口

    for elements in elements.windows(2) {
        if let [a, b] = elements {
            pipeline.add(b).map_err(|_| "Cannot add elements to pipeline")?; // 将元素添加到管道中
            a.link(b).map_err(|_| "Cannot link elements")?; // 连接两个素
        }
    }

    let sinkpad = elements.first().unwrap().static_pad("sink").unwrap(); // 获取第一个元素的输入端口
    teepad.link(&sinkpad).map_err(|_| "Cannot link the pad of output tee to the pad of first element")?; // 连输出分支端和第一个素的输入端

    output_tee.sync_state_with_parent().unwrap(); // 同步输出支的状态与父对象

    for element in elements {
        element.sync_state_with_parent().unwrap(); // 同每个元素的状态与父对象
 }

    Ok((output_tee, teepad)) // 返回输出分支和连接的端口
}

pub fn disconnect_elements_to_pipeline(pipeline: &Pipeline, (output_tee, teepad): &(Element, Pad), elements: &[Element]) -> Result<Future<()>, String> {
    // 断开连接元素与管道之间的链接
    let first_sinkpad = elements.first().unwrap().static_pad("sink").unwrap();
    // 解除连接并返回错误信息，如果无法解除连接
    teepad.unlink(&first_sinkpad).map_err(|_| "无法解除元素之间的连接")?;
    // 从输出分流器中移除端口，并返回错误信息如果无法移除端口
    output_tee.remove_pad(teepad).map_err(|_| "无法从输出分流器中移除端口")?;
    // 获取最后一个元素的接收端口
    let last_sinkpad = elements.last().unwrap().sink_pads().into_iter().next().unwrap();
    // 将素列表转换为可变向量
    let elements = elements.to_vec();
    // 创建一个 Promise 对象
    let promise = Promise::new();
    // 获取 Promise 对应的 Future
    let future = promise.future();
    // 使用互斥锁包装 Promise 对象
    let promise = Mutex::new(Some(promise));
    // 添加事件探测器到最后一个元素的接收端口
    last_sinkpad.add_probe(PadProbeType::EVENT_BOTH, move |_pad, info| {
    match &info.data {
    Some(PadProbeData::Event(event)) => {
    if let EventView::Eos(_) = event.view() {
    // 如果是 End of Stream 事件，则标记 Promise 成功，并返回 Remove
    promise.lock().unwrap().take().unwrap().success(());
    PadProbeReturn::Remove
    } else {
    // 否则返回 Pass
    PadProbeReturn::Pass
    }
    },
    _ => PadProbeReturn::Pass,
    }
    });
    // 发送 End of Stream 事件到第一个元的接收端
    first_sinkpad.send_event(gst::event::Eos::new());
    // 使用克隆闭包将 pipeline 强引用传递给 Future 的回调函数
    let future = future.map(clone!(@strong pipeline => move |_| {
    // 从道中移除多个元素，并返回错误信息如果无法移除元素
    pipeline.remove_many(&elements.iter().collect::<Vec<_>>()).map_err(|_| "Cannot remove elements from pipeline").unwrap();
    // 将个元素设置为 Null 状态，并返回错误信息，如果无设置状态
    for element in elements.iter() {
    element.set_state(gst::State::Null).unwrap();
    }
    }));
    // 返回 Future 对象
    Ok(future)
    }

pub fn create_pipeline(url: &Url) -> Result<gst::Pipeline, String> {
    let pipeline = gst::Pipeline::new(None);
    // 定义一个函数，用于获取源元素列表
    fn gst_src_elements(url: &Url) -> Result<Vec<Element>, String> {
        let mut elements = Vec::new();
        // 创建 rtspsrc 元素并设置属性
        let rtspsrc = gst::ElementFactory::make("rtspsrc", Some("source")).map_err(|_| "Missing element: rtspsrc")?;
        rtspsrc.set_property("location", url.to_string());
        rtspsrc.set_property("user-id", url.username());
        if let Some(password) = url.password() {
            rtspsrc.set_property("user-pw", password);
        }
        rtspsrc.set_property("latency", 0u32);
        elements.push(rtspsrc);
        // 创建 depay 元素并添加到列表中
        let depay = gst::ElementFactory::make("rtph265depay", Some("rtpdepay")).map_err(|_| format!("Missing element: {}", "rtph265depay"))?;
        elements.push(depay);
        Ok(elements)
    }
    // 获取源元素列表
    let src_elements = gst_src_elements(url)?;
    
    // 分离视频源和 depay 元素
    let (video_src, depay_elements) = src_elements.split_first().ok_or_else(|| "Source element is empty")?;
    let video_src = video_src.clone();
    // 创建 appsink 元素并设置属性
    let appsink = gst::ElementFactory::make("appsink", Some("display")).map_err(|_| "Missing element: appsink")?;
    let caps_app = gst::caps::Caps::from_str("video/x-raw, format=RGB").map_err(|_| "Cannot create capability for appsink")?;
    appsink.set_property("caps", caps_app);
 // 创建 tee 元素
    let tee_source = gst::ElementFactory::make("tee", Some("tee_source")).map_err(|_| "Missing element: tee")?;
    let tee_decoded = gst::ElementFactory::make("tee", Some("tee_decoded")).map_err(|_| "Missing element: tee")?;
    // 创建 queue 元素
    let queue_to_decode = gst::ElementFactory::make("queue", None).map_err(|_| "Missing element: queue")?;
    let queue_to_app = gst::ElementFactory::make("queue", None).map_err(|_| "Missing element: queue")?;
    // 定义一个函数，用于获取元素列表
    fn gst_elements() -> Result<Vec<Element>, String> {
        Ok(vec![gst::ElementFactory::make("videoconvert", None).map_err(|_| "Missing element: videoconvert")?])
    }
    // 获取颜色空间换元素列表
    let colorspace_conversion_elements = gst_elements()?;
    // 定义一个函数，用于获取主要元素列表
    fn gst_main_elements() -> Result<Vec<Element>, String> {
        let mut elements = Vec::new();
        // 创建 parse 元素并添加到列表中
        let parse = gst::ElementFactory::make("h265parse", None).map_err(|_| "Missing element: h265parse")?;
        elements.push(parse);
        let decoder_name = "avdec_h265";
        // 创建解码元素并添加到列表中
        let decoder = gst::ElementFactory::make(&decoder_name, Some("video_decoder")).map_err(|_| format!("Missing element: {}", &decoder_name))?;
        elements.push(decoder);
        Ok(elements)
    }
    // 获取解器元素列表
    let decoder_elements = gst_main_elements()?;
    
    // 向管道添加多个元素：video_src、appsink、tee_decoded、tee_source、queue_to_app、queue_to_decode，并返回错误信息"Cannot create pipeline"，如果添加失败。
    pipeline.add_many(&[&video_src, &appsink, &tee_decoded, &tee_source, &queue_to_app, &queue_to_decode]).map_err(|_| "无法创建管道")?;

    // 向管道添加颜色间转换元素的集合colorspace_conversion_elements，并返回错误信息"Cannot add colorspace conversion elements to pipeline"，如果添加失败。
    pipeline.add_many(&colorspace_conversion_elements.iter().collect::<Vec<_>>()).map_err(|_| "无法将颜色空间转换元素添加到管道")?;

    // 遍历depay_elements中的每个元素，将其添加到管道，并返回错误信息"Cannot add depay elements to pipeline"如果添加失败。
    for depay_element in depay_elements {
        pipeline.add(depay_element).map_err(|_| "无将depay元素添加到管道")?;
    }

    // 遍历decoder_elements中的每个元素，将其添加到管道，并返回错误信息"Cannot add decoder elements element"，如果添加失败。
    for decoder_element in &decoder_elements {
        pipeline.add(decoder_element).map_err(|_| "无法将解码器元素添加到管道")?;
    }

    // 遍历depay_elements中的每个相邻元素，将它们链接起来，并返回错误信息"Cannot link elements between depay elements"，如果链接失败。
    for element in depay_elements.windows(2) {
        if let [a, b] = element {
            a.link(b).map_err(|_| "无法在de元素之间建立链接")?;
        }
    }

    // 遍历decoder_elements的每两个邻元素，将它们链接起来，并返回错误信息"Cannot link elements between decoder elements"，如果链接失败。
    for element in decoder_elements.windows(2) {
        if let [a, b] = element {
            a.link(b).map_err(|_| "无法解码器元素之建立链接")?;
        }
    }

    // 遍历colorspace_conversion_elements中的每两个相邻元素，将它们链接起来，并返回错误信息"Cannot link elements between colorspace conversion elements"，如果链接失败。
    for element in colorspace_conversion_elements.windows(2) {
        if let [a, b] = element {
            a.link(b).map_err(|_| "无法在色空间转元素之间建链接")?;
        }
    }
    // 匹配解码器元素的第一个和最后一个
    match (decoder_elements.first(), decoder_elements.last()) {
        (Some(first), Some(last)) => {
            // 将队列与第一个解码器元素连接起来
            queue_to_decode.link(first).map_err(|_| "无法将队列链接到第一个解码器元素")?;
            // 将最后一个解码连接到tee_decoded
            last.link(&tee_decoded).map_err(|_| "无法将最后一个解码连接到tee")?;
        },
        _ => return Err("缺少解码器元素".to_string()),
    }

    // 匹配颜色空间转换素的第一个最后一个
    match (colorspace_conversion_elements.first(), colorspace_conversion_elements.last()) {
        (Some(first), Some(last)) => {
            // 将队列与第一个颜色空间转换元素连接起来
            queue_to_app.link(first).map_err(|_| "无法将最后一个解码元素链接到第一个颜色空间转换元")?;
            // 将最后一个颜色空间转换元素连接到appsink
            last.link(&appsink).map_err(|_| "无法将后一个颜色空间转换元素链接到appsink")?;
        },
        _ => return Err("缺少解码器元素".to_string()),
    }

    // 设置queue_to_app的属性为"leaky"
    queue_to_app.set_property_from_value("leaky", &EnumClass::new(queue_to_app.property_type("leaky").unwrap()).unwrap().to_value(2).unwrap());

    // 请求tee_source的简单源，并将其与queue_to_decode的静态sink pad连接起来
    tee_source.request_pad_simple("src_%u").unwrap().link(&queue_to_decode.static_pad("sink").unwrap()).map_err(|_| "无法将tee链接到解码器队列")?;

    // 请求tee_decoded的简源pad，并将与queue_to_app的静态sink pad连接起来`
    tee_decoded.request_pad_simple("src_%u").unwrap().link(&queue_to_app.static_pad("sink").unwrap()).map_err(|_| "法将tee链接到appsink队列")?;

    // 匹配第一个和最后一个元素
    match (depay_elements.first(), depay_elements.last()) {
        (Some(first), Some(last)) => {
            let first = first.clone();

            // 如果video_src的"src" pad存在
            if let Some(src) = video_src.static_pad("src") {
                // 将src与第一个depay素的"sink" pad连接起来，连接失败则报
                src.link(&first.static_pad("sink").unwrap()).map_err(|_| "Cannot link video source element to the first depay element").unwrap();
            } else {
                // 当video_src的pad被添加时执行以下操作
                video_src.connect("pad-added", true, move |args| {
                    if let [_element, pad] = args {
                        let pad = pad.get::<Pad>().unwrap();
                        // 获取pad的媒体类型
                        let media = pad.caps().unwrap().iter().flat_map(|x| x.iter()).find_map(|(key, value)| {
                            if key == "media" {
                                Some(value.get::<String>().unwrap())
                            } else {
                                None
                            }
                        });
                        
                         // 如果媒体类型为视频，则将pad与第一个de元素的"sink" pad连接起来，连接失败则报错
                        if media.map_or(false, |x| x.eq("video")) {
                            pad.link(&first.static_pad("sink").unwrap()).map_err(|_| "Cannot delay link video source element to the first depay element").unwrap();
                        }
                    }
                    None
                });
            }
            // 将最后一个depay元与tee_source连接起来，连接失败则报错
            last.link(&tee_source).map_err(|_| "Cannot link the last depay element to tee")?;
        },
        _ => video_src.link(&tee_source).map_err(|_| "Cannot link video source to tee")?,
    }
    Ok(pipeline)
}

fn correct_underwater_color(src: Mat) -> Mat {
    // 创建一个默认的图像对象
    let mut image = Mat::default();
    
    // 将源图像转换为32位浮点型，范围为[0, 1]
    src.convert_to(&mut image, cv::core::CV_32FC3, 1.0, 0.0).expect("无法转换源图像");
    
    // 将图像素值缩放到[0, 1]范围内
    let image = (image / 255.0).into_result().unwrap();
    
    // 创建一个存储道图像的量
    let mut channels = cv::types::VectorOfMat::new();
    
    // 将图像拆分成通图像
    cv::core::split(&image, &mut channels).expect("法拆分图像");
    
    // 创建均值和标准差变量
    let [mut mean, mut std] = [cv::core::Scalar::default(); 2];
    
    // 保存始图像尺寸
    let image_original_size = image;
    
    // 创建一个新的图像对象，并将原始图像调整为128x128大小
    let mut image = Mat::default();
    cv::imgproc::resize(&image_original_size, &mut image, Size::new(128, 128), 0.0, 0.0, imgproc::INTER_NEAREST).expect("Cannot resize image");
    
    // 计算图像的均值标准差
    cv::core::mean_std_dev(&image, &mut mean, &mut std, &cv::core::no_array()).expect("无法计算图的均值和标准差");
    
    // 定义常量U
    const U: f64 = 3.0;
    
    // 计算每个通道的最小值最大值
    let min_max = mean.iter().zip(std.iter()).map(|(mean, std)| (mean - U * std, mean + U * std));
    
    // 对每个通道进行归一化处理，将像素值缩放到[0, 255]范围内
    let channels = channels.iter().zip(min_max).map(|(channel, (min, max))| (channel - VecN::from(min)) / (max - min) * 255.0).map(|x| x.into_result().and_then(|x| x.to_mat()).unwrap());
    
    // 将归一化后的通道图像重新组合成一个图像对象
    let channels = VectorOfMat::from_iter(channels);
    let mut image = Mat::default();
    cv::core::merge(&channels, &mut image).expect("法合并通道图像");
    
    // 创建结果图像对象
    let mut result = Mat::default();
    
    // 将图像数据类型转为8位无符号整型
    image.convert_to(&mut result, cv::core::CV_8UC3, 1.0, 0.0).expect("无法转换结果数据类型");
    
    // 返回结果像
    result
}

#[allow(dead_code)]
fn apply_clahe(mut mat: Mat) -> Mat {
    // 将输入图像拆分为通道
    let mut channels = VectorOfMat::new();
    cv::core::split(&mat, &mut channels).expect("Cannot split image");
    // 创建CLAHE对象并应用于每个道
    if let Ok(mut clahe) = imgproc::create_clahe(2.0, Size::new(8, 8)) {
        for mut channel in channels.iter() {
            clahe.apply(&channel.clone(), &mut channel).expect("Cannot apply CLAHE");
        }
    }
    // 合并处理后的通道
    cv::core::merge(&channels, &mut mat).expect("Cannot merge result channels");
    mat
}

pub fn attach_pipeline_callback(pipeline: &Pipeline, sender: Sender<Mat>, config: Arc<Mutex<SlaveConfigModel>>) -> Result<(), String> {
    // 创建一个用于存储帧大小的共享变量
    let frame_size: Arc<Mutex<Option<(i32, i32)>>> = Arc::new(Mutex::new(None));
    // 获取名为"display"的元素，并将其换为AppSink类型
    let appsink = pipeline.by_name("display").unwrap().dynamic_cast::<gst_app::AppSink>().unwrap();
    // 设置AppSink的回调函数
    appsink.set_callbacks(
        gst_app::AppSinkCallbacks::builder()
            .new_event(clone!(@strong frame_size => move |appsink| {
                if let Ok(miniobj) = appsink.pull_object() {
                    if let Ok(event) = miniobj.downcast::<gst::Event>() {
                        if let EventView::Caps(caps) = event.view() {
                            let caps = caps.caps();
                            if let Some(structure) = caps.structure(0) {
                                match (structure.get("width"), structure.get("height")) {
                                    (Ok(width), Ok(height)) => {
                                        *frame_size.lock().unwrap() = Some((width, height));
                                    },
                                    _ => (),
                                }
                            }
                        }
                    }
                }
                true
            }))
            .new_sample(clone!(@strong frame_size => move |appsink| {
                // 获取帧大小
                let (width, height) = frame_size.lock().unwrap().ok_or(gst::FlowError::Flushing)?;
                // 从AppSink中获取样本
                let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                // 从样本中获取缓冲区
                let buffer = sample.buffer().ok_or_else(|| {
                    element_error!(
                        appsink,
                        gst::ResourceError::Failed,
                        ("Failed to get buffer from appsink")
                    );
                    gst::FlowError::Error
                })?;
                // 将缓冲区映射可读取的数据
                let map = buffer.map_readable().map_err(|_| {
                    element_error!(
                        appsink,
                        gst::ResourceError::Failed,
                        ("Failed to map readable buffer")
                    );
                    gst::FlowError::Error
                })?;
                // 创建一个OpenCV的Mat对象，将映射的数据作像素数据
                let mat = unsafe {
                    Mat::new_rows_cols_with_data(height, width, cv::core::CV_8UC3, map.as_ptr() as *mut c_void, cv::core::Mat_AUTO_STEP)
                }.map_err(|_| gst::FlowError::CustomError)?.clone();
                // 根据配置对Mat对象进行处理
                let mat = match config.lock() {
                    Ok(config) => {
                        match config.video_algorithms.first() {
                            Some(VideoAlgorithm::CLAHE) => {
                                apply_clahe(correct_underwater_color(mat))
                            },
                            _ => mat,
                        }
                    },
                    Err(_) => mat,
                };
                // 将处理后的Mat对象发送给接收者
                sender.send(mat).unwrap();
                Ok(gst::FlowSuccess::Ok)
            }))
            .build());
    Ok(())
}

pub trait MatExt {
    fn as_pixbuf(&self) -> Pixbuf;
}

impl MatExt for Mat {
    fn as_pixbuf(&self) -> Pixbuf {
        // 将图像转换为 Pixbuf 类型
        let width = self.cols();  // 获取图像的宽度
        let height = self.rows();  // 获取图像的高度
        let size = (width * height * 3) as usize;  // 计算图像数据的大小
        let pixbuf = Pixbuf::new(Colorspace::Rgb, false, 8, width, height).unwrap();  // 创建一个新的 Pixbuf 对象
        unsafe {
            pixbuf.pixels()[..size].copy_from_slice(self.data_bytes().unwrap());  // 将图像数据复制到 Pixbuf 对象中
        }
        pixbuf  // 返回生成的 Pixbuf 对象
    }
}

