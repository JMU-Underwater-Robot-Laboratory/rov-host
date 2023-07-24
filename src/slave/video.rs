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
    let output_tee = pipeline.by_name(tee_name).ok_or("Cannot find output_tee")?;
    if let Some(element) = elements.first() {
        pipeline.add(element).map_err(|_| "Cannot add the first element to pipeline")?; // 必须先添加，再连接
    }
    let teepad = output_tee.request_pad_simple("src_%u").ok_or("Cannot request pad")?;
    for elements in elements.windows(2) {
        if let [a, b] = elements {
            pipeline.add(b).map_err(|_| "Cannot add elements to pipeline")?;
            a.link(b).map_err(|_| "Cannot link elements")?;
        }
    }
    let sinkpad = elements.first().unwrap().static_pad("sink").unwrap();
    teepad.link(&sinkpad).map_err(|_| "Cannot link the pad of output tee to the pad of first element")?;
    output_tee.sync_state_with_parent().unwrap();
    for element in elements {
        element.sync_state_with_parent().unwrap();
    }
    Ok((output_tee, teepad))
}

pub fn disconnect_elements_to_pipeline(pipeline: &Pipeline, (output_tee, teepad): &(Element, Pad), elements: &[Element]) -> Result<Future<()>, String> {
    let first_sinkpad = elements.first().unwrap().static_pad("sink").unwrap();
    teepad.unlink(&first_sinkpad).map_err(|_| "Cannot unlink elements")?;
    output_tee.remove_pad(teepad).map_err(|_| "Cannot remove pad from output tee")?;
    let last_sinkpad = elements.last().unwrap().sink_pads().into_iter().next().unwrap();
    let elements = elements.to_vec();
    let promise = Promise::new();
    let future = promise.future();
    let promise = Mutex::new(Some(promise));
    last_sinkpad.add_probe(PadProbeType::EVENT_BOTH, move |_pad, info| {
        match &info.data {
            Some(PadProbeData::Event(event)) => {
                if let EventView::Eos(_) = event.view() {
                    promise.lock().unwrap().take().unwrap().success(());
                    PadProbeReturn::Remove
                } else {
                    PadProbeReturn::Pass
                }
            },
            _ => PadProbeReturn::Pass,
        }
    });
    first_sinkpad.send_event(gst::event::Eos::new());
    let future = future.map(clone!(@strong pipeline => move |_| {
        pipeline.remove_many(&elements.iter().collect::<Vec<_>>()).map_err(|_| "Cannot remove elements from pipeline").unwrap();
        for element in elements.iter() {
            element.set_state(gst::State::Null).unwrap();
        }
    }));
    Ok(future)
}

pub fn create_pipeline(url: &Url) -> Result<gst::Pipeline, String> {
    let pipeline = gst::Pipeline::new(None);
    fn gst_src_elements(url: &Url) -> Result<Vec<Element>, String> {
        let mut elements = Vec::new();
                let rtspsrc = gst::ElementFactory::make("rtspsrc", Some("source")).map_err(|_| "Missing element: rtspsrc")?;
                rtspsrc.set_property("location", url.to_string());
                rtspsrc.set_property("user-id", url.username());
                if let Some(password) = url.password() {
                    rtspsrc.set_property("user-pw", password);
                }
                rtspsrc.set_property("latency", 0u32);
                elements.push(rtspsrc);
                let depay = gst::ElementFactory::make("rtph265depay", Some("rtpdepay")).map_err(|_| format!("Missing element: {}", "rtph265depay"))?;
                elements.push(depay);
        Ok(elements)
    }
    let src_elements = gst_src_elements(url)?;
    
    let (video_src, depay_elements) = src_elements.split_first().ok_or_else(|| "Source element is empty")?;
    let video_src = video_src.clone();
    let appsink = gst::ElementFactory::make("appsink", Some("display")).map_err(|_| "Missing element: appsink")?;
    let caps_app = gst::caps::Caps::from_str("video/x-raw, format=RGB").map_err(|_| "Cannot create capability for appsink")?;
    appsink.set_property("caps", caps_app);
    let tee_source = gst::ElementFactory::make("tee", Some("tee_source")).map_err(|_| "Missing element: tee")?;
    let tee_decoded = gst::ElementFactory::make("tee", Some("tee_decoded")).map_err(|_| "Missing element: tee")?;
    let queue_to_decode = gst::ElementFactory::make("queue", None).map_err(|_| "Missing element: queue")?;
    let queue_to_app = gst::ElementFactory::make("queue", None).map_err(|_| "Missing element: queue")?;
    fn gst_elements() -> Result<Vec<Element>, String> {
            Ok(vec![gst::ElementFactory::make("videoconvert", None).map_err(|_| "Missing element: videoconvert")?])
    }
    let colorspace_conversion_elements = gst_elements()?;
    fn gst_main_elements() -> Result<Vec<Element>, String> {
        let mut elements = Vec::new();
        let parse = gst::ElementFactory::make("h265parse", None).map_err(|_| "Missing element: h265parse")?;
        elements.push(parse);
        let decoder_name = "avdec_h265";
        let decoder = gst::ElementFactory::make(&decoder_name, Some("video_decoder")).map_err(|_| format!("Missing element: {}", &decoder_name))?;
        elements.push(decoder);
        Ok(elements)
    }
    let decoder_elements = gst_main_elements()?;
    
    pipeline.add_many(&[&video_src, &appsink, &tee_decoded, &tee_source, &queue_to_app, &queue_to_decode]).map_err(|_| "Cannot create pipeline")?;
    pipeline.add_many(&colorspace_conversion_elements.iter().collect::<Vec<_>>()).map_err(|_| "Cannot add colorspace conversion elements to pipeline")?;
    for depay_element in depay_elements {
        pipeline.add(depay_element).map_err(|_| "Cannot add depay elements to pipeline")?;
    }
    for decoder_element in &decoder_elements {
        pipeline.add(decoder_element).map_err(|_| "Cannot add decoder elements element")?;
    }
    for element in depay_elements.windows(2) {
        if let [a, b] = element {
            a.link(b).map_err(|_| "Cannot link elements between depay elements")?;
        }
    }
    for element in decoder_elements.windows(2) {
        if let [a, b] = element {
            a.link(b).map_err(|_| "Cannot link elements between decoder elements")?;
        }
    }
    for element in colorspace_conversion_elements.windows(2) {
        if let [a, b] = element {
            a.link(b).map_err(|_| "Cannot link elements between colorspace conversion elements")?;
        }
    }
    match (decoder_elements.first(), decoder_elements.last()) {
        (Some(first), Some(last)) => {
            queue_to_decode.link(first).map_err(|_| "Cannot link queue to the first decoder element")?;
            last.link(&tee_decoded).map_err(|_| "Cannot link last decode to tee")?;
        },
        _ => return Err("Missing decoder element".to_string()),
    }
    match (colorspace_conversion_elements.first(), colorspace_conversion_elements.last()) {
        (Some(first), Some(last)) => {
            queue_to_app.link(first).map_err(|_| "Cannot link the last decoder element to first colorspace conversion element")?;
            last.link(&appsink).map_err(|_| "Cannot link last colorspace conversion element to appsink")?;
        },
        _ => return Err("Missing decoder element".to_string()),
    }
    queue_to_app.set_property_from_value("leaky", &EnumClass::new(queue_to_app.property_type("leaky").unwrap()).unwrap().to_value(2).unwrap());

    tee_source.request_pad_simple("src_%u").unwrap().link(&queue_to_decode.static_pad("sink").unwrap()).map_err(|_| "Cannot link tee to decoder queue")?;
    tee_decoded.request_pad_simple("src_%u").unwrap().link(&queue_to_app.static_pad("sink").unwrap()).map_err(|_| "Cannot link tee to appsink queue")?;
    match (depay_elements.first(), depay_elements.last()) {
        (Some(first), Some(last)) => {
            let first = first.clone();
            if let Some(src) = video_src.static_pad("src") {
                src.link(&first.static_pad("sink").unwrap()).map_err(|_| "Cannot link video source element to the first depay element").unwrap();
            } else {
                video_src.connect("pad-added", true, move |args| {
                    if let [_element, pad] = args {
                        let pad = pad.get::<Pad>().unwrap();
                        let media = pad.caps().unwrap().iter().flat_map(|x| x.iter()).find_map(|(key, value)| {
                            if key == "media" {
                                Some(value.get::<String>().unwrap())
                            } else {
                                None
                            }
                        });
                        
                        if media.map_or(false, |x| x.eq("video")) {
                            pad.link(&first.static_pad("sink").unwrap()).map_err(|_| "Cannot delay link video source element to the first depay element").unwrap();
                        }
                    }
                    None
                });
            }
            last.link(&tee_source).map_err(|_| "Cannot link the last depay element to tee")?;
        },
        _ => video_src.link(&tee_source).map_err(|_| "Cannot link video source to tee")?,
    }
    Ok(pipeline)
}

fn correct_underwater_color(src: Mat) -> Mat {
    let mut image = Mat::default();
    src.convert_to(&mut image, cv::core::CV_32FC3, 1.0, 0.0).expect("Cannot convert source image");
    let image = (image / 255.0).into_result().unwrap();
    let mut channels = cv::types::VectorOfMat::new();
    cv::core::split(&image, &mut channels).expect("Cannot split image");
    let [mut mean, mut std] = [cv::core::Scalar::default(); 2];
    let image_original_size = image;
    let mut image = Mat::default();
    cv::imgproc::resize(&image_original_size, &mut image, Size::new(128, 128), 0.0, 0.0, imgproc::INTER_NEAREST).expect("Cannot resize image");
    cv::core::mean_std_dev(&image, &mut mean, &mut std, &cv::core::no_array()).expect("Cannot calculate mean and standard deviation for image");
    const U: f64 = 3.0;
    let min_max = mean.iter().zip(std.iter()).map(|(mean, std)| (mean - U * std, mean + U * std));
    let channels = channels.iter().zip(min_max).map(|(channel, (min, max))| (channel - VecN::from(min)) / (max - min) * 255.0).map(|x| x.into_result().and_then(|x| x.to_mat()).unwrap());
    let channels = VectorOfMat::from_iter(channels);
    let mut image = Mat::default();
    cv::core::merge(&channels, &mut image).expect("Cannot merge result channels");
    let mut result = Mat::default();
    image.convert_to(&mut result, cv::core::CV_8UC3, 1.0, 0.0).expect("Cannot convert result data type");
    result
}

#[allow(dead_code)]
fn apply_clahe(mut mat: Mat) -> Mat {
    let mut channels = VectorOfMat::new();
    cv::core::split(&mat, &mut channels).expect("Cannot split image");
    if let Ok(mut clahe) = imgproc::create_clahe(2.0, Size::new(8, 8)) {
        for mut channel in channels.iter() {
            clahe.apply(&channel.clone(), &mut channel).expect("Cannot apply CLAHE");
        }
    }
    cv::core::merge(&channels, &mut mat).expect("Cannot merge result channels");
    mat
}

pub fn attach_pipeline_callback(pipeline: &Pipeline, sender: Sender<Mat>, config: Arc<Mutex<SlaveConfigModel>>) -> Result<(), String> {
    let frame_size: Arc<Mutex<Option<(i32, i32)>>> = Arc::new(Mutex::new(None));
    let appsink = pipeline.by_name("display").unwrap().dynamic_cast::<gst_app::AppSink>().unwrap();
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
                let (width, height) = frame_size.lock().unwrap().ok_or(gst::FlowError::Flushing)?;
                let sample = appsink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                let buffer = sample.buffer().ok_or_else(|| {
                    element_error!(
                        appsink,
                        gst::ResourceError::Failed,
                        ("Failed to get buffer from appsink")
                    );
                    gst::FlowError::Error
                })?;
                let map = buffer.map_readable().map_err(|_| {
                    element_error!(
                        appsink,
                        gst::ResourceError::Failed,
                        ("Failed to map readable buffer")
                    );
                    gst::FlowError::Error
                })?;
                let mat = unsafe {
                    Mat::new_rows_cols_with_data(height, width, cv::core::CV_8UC3, map.as_ptr() as *mut c_void, cv::core::Mat_AUTO_STEP)
                }.map_err(|_| gst::FlowError::CustomError)?.clone();
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
        let width = self.cols();
        let height = self.rows();
        let size = (width * height * 3) as usize;
        let pixbuf = Pixbuf::new(Colorspace::Rgb, false, 8, width, height).unwrap();
        unsafe {
            pixbuf.pixels()[..size].copy_from_slice(self.data_bytes().unwrap());
        }
        pixbuf
    }
}

