/* slave_video.rs
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

use std::{
    cell::RefCell,
    fmt::Debug,
    path::PathBuf,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use adw::StatusPage;
use gdk_pixbuf::Pixbuf;
use glib::{clone, MainContext, Sender};
use gst::{prelude::*, Element, Pipeline};
use gtk::{prelude::*, Box as GtkBox, Picture, Stack};
use relm4::{send, MicroModel, MicroWidgets};
use relm4_macros::micro_widget;

use derivative::*;

use super::{slave_config::SlaveConfigModel, SlaveMsg};
use crate::{
    async_glib::{Future, Promise},
    preferences::PreferencesModel,
    slave::video::MatExt,
};

#[tracker::track]
#[derive(Debug, Derivative)]
#[derivative(Default)]
pub struct SlaveVideoModel {
    #[no_eq]
    pub pixbuf: Option<Pixbuf>,
    #[no_eq]
    pub pipeline: Option<Pipeline>,
    #[no_eq]
    pub config: Arc<Mutex<SlaveConfigModel>>,
    pub record_handle: Option<((gst::Element, gst::Pad), Vec<gst::Element>)>,
    #[derivative(Default(value = "Rc::new(RefCell::new(PreferencesModel::load_or_default()))"))]
    pub preferences: Rc<RefCell<PreferencesModel>>,
}

impl SlaveVideoModel {
    pub fn new(
        preferences: Rc<RefCell<PreferencesModel>>,
        config: Arc<Mutex<SlaveConfigModel>>,
    ) -> Self {
        SlaveVideoModel {
            preferences,
            config,
            ..Default::default()
        }
    }
    pub fn is_running(&self) -> bool {
        self.pipeline.is_some()
    }

    pub fn is_recording(&self) -> bool {
        self.record_handle.is_some()
    }
}

pub enum SlaveVideoMsg {
    StartPipeline,
    StopPipeline,
    SetPixbuf(Option<Pixbuf>),
    StartRecord(PathBuf),
    StopRecord(Option<Promise<()>>),
    ConfigUpdated(SlaveConfigModel),
    SaveScreenshot(PathBuf),
    RequestFrame,
}

impl MicroModel for SlaveVideoModel {
    type Msg = SlaveVideoMsg;
    type Widgets = SlaveVideoWidgets;
    type Data = Sender<SlaveMsg>;

    fn update(
        &mut self,
        msg: SlaveVideoMsg,
        parent_sender: &Sender<SlaveMsg>,
        sender: Sender<SlaveVideoMsg>,
    ) {
        self.reset();
        match msg {
            SlaveVideoMsg::SetPixbuf(pixbuf) => {
                if self.get_pixbuf().is_none() {
                    send!(parent_sender, SlaveMsg::PollingChanged(true)); // 主要是更新截图按钮的状态
                }
                self.set_pixbuf(pixbuf)
            }
            SlaveVideoMsg::StartRecord(pathbuf) => {
                if let Some(pipeline) = &self.pipeline {
                    // 定义一个公共函数，用于创建配置录制所需元素
                    pub fn gst_record_elements(filename: &str) -> Result<Vec<Element>, String> {
                        let mut elements = Vec::new();

                        // 创建并添加队列元素
                        let queue_to_file = gst::ElementFactory::make("queue", None)
                            .map_err(|_| "缺少元素：queue")?;
                        elements.push(queue_to_file);

                        // 创建并添加H265解析器元素
                        let parse = gst::ElementFactory::make("h265parse", None)
                            .map_err(|_| "缺少元素：h265parse")?;
                        elements.push(parse);

                        // 创建并添加Matroska复用器元素
                        let matroskamux = gst::ElementFactory::make("matroskamux", None)
                            .map_err(|_| "缺少复用器：matroskamux")?;
                        elements.push(matroskamux);

                        // 创建并添加文件输出元素
                        let filesink = gst::ElementFactory::make("filesink", None)
                            .map_err(|_| "缺少元素：filesink")?;
                        filesink.set_property("location", filename);
                        elements.push(filesink);

                        Ok(elements)
                    }

                    // 获取录制处理句柄
                    let record_handle = {
                        let elements = gst_record_elements(&pathbuf.to_str().unwrap());
                        let elements_and_pad = elements.and_then(|elements| {
                            super::video::connect_elements_to_pipeline(
                                pipeline,
                                "tee_source",
                                &elements,
                            )
                            .map(|pad| (elements, pad))
                        });
                        elements_and_pad
                    };

                    // 处理录制处理句柄的结果
                    match record_handle {
                        Ok((elements, pad)) => {
                            // 将录制处理句柄保存到self.record_handle中，并发送RecordingChanged消息给父进程
                            self.record_handle = Some((pad, Vec::from(elements)));
                            send!(parent_sender, SlaveMsg::RecordingChanged(true));
                        }
                        Err(err) => {
                            // 发送ErrorMessage和RecordingChanged消息给父进程，表示录制处理失败
                            send!(parent_sender, SlaveMsg::ErrorMessage(err.to_string()));
                            send!(parent_sender, SlaveMsg::RecordingChanged(false));
                        }
                    }
                }
            }
            SlaveVideoMsg::StopRecord(promise) => {
                if let Some(pipeline) = &self.pipeline {
                    // 如果存在 pipeline
                    if let Some((teepad, elements)) = &self.record_handle {
                        // 如果存在 record_handle
                        super::video::disconnect_elements_to_pipeline(pipeline, teepad, elements)
                            .unwrap()
                            .for_each(clone!(@strong parent_sender => move |_| {
                                // 断开元素与 pipeline 的连接，并每个断开的元素执行以下操作
                                send!(parent_sender, SlaveMsg::RecordingChanged(false));
                                // 向父发送者发送消息，表示录制状态已更改 false
                                if let Some(promise) = promise {
                                    // 如果存在 promise
                                    promise.success(());
                                    // 完成 promise
                                }
                            }));
                    }
                    self.set_record_handle(None);
                    // 清空 record_handle
                }
            }
            SlaveVideoMsg::ConfigUpdated(config) => {
                *self.get_mut_config().lock().unwrap() = config;
            }
            SlaveVideoMsg::StartPipeline => {
                assert!(self.pipeline == None);
                // 断言 self.pipeline 为 None

                let config = self.get_config().lock().unwrap();
                // 获取配置并进行锁定

                let video_url = config.get_video_url();
                // 获取视频 URL

                match super::video::create_pipeline(video_url) {
                    // 创建 pipeline
                    Ok(pipeline) => {
                        drop(config);
                        // 释放配置的锁

                        let sender = sender.clone();
                        // 克隆发送者
                        let (mat_sender, mat_receiver) =
                            MainContext::channel(glib::PRIORITY_DEFAULT);
                        // 创建道，用于传输图像数据

                        super::video::attach_pipeline_callback(
                            &pipeline,
                            mat_sender,
                            self.get_config().clone(),
                        )
                        .unwrap();
                        // 将回调函数附到 pipeline 上，并递相关参数

                        mat_receiver.attach(None, move |mat| {
                            // 监听图像数据接收器，并对每个接收到的图像执行以下操作
                            sender
                                .send(SlaveVideoMsg::SetPixbuf(Some(mat.as_pixbuf())))
                                .unwrap();
                            // 发送 SlaveVideo::SetPixbuf 消息，将图数据作为 Pixbuf 发送发送者
                            Continue(true)
                            // 继续监听
                        });

                        match pipeline.set_state(gst::State::Playing) {
                            // 设置 pipeline 的状态为 Playing
                            Ok(_) => {
                                self.set_pipeline(Some(pipeline));
                                // 设置 self.pipeline 为 Some(pipeline)
                                send!(parent_sender, SlaveMsg::PollingChanged(true));
                                // 向父发送者发送消息，表示轮询状态已更改为 true
                            }
                            Err(_) => {
                                send!(parent_sender, SlaveMsg::ErrorMessage(String::from("无法启管道，这可能是由于管道使用的资源不存在或占用导致的，请检查相关资源是否用。")));
                                // 向发送者发送错误消息，表示无法启动管道
                                send!(parent_sender, SlaveMsg::PollingChanged(false));
                                // 向父发送者发送消息，表示轮询状态已更为 false
                            }
                        }
                    }
                    Err(msg) => {
                        send!(parent_sender, SlaveMsg::ErrorMessage(String::from(msg)));
                        // 向父发送者发送错误消息，表示创建管道失败
                        send!(parent_sender, SlaveMsg::PollingChanged(false));
                        // 向父发送发送消息，表示轮询状态已更改为 false
                    }
                }
            }
            SlaveVideoMsg::StopPipeline => {
                assert!(self.pipeline != None);
                let mut futures = Vec::<Future<()>>::new();
                let recording = self.is_recording();
                if recording {
                    let promise = Promise::new();
                    let future = promise.future();
                    self.update(
                        SlaveVideoMsg::StopRecord(Some(promise)),
                        parent_sender,
                        sender.clone(),
                    );
                    futures.push(future);
                }
                let promise = Promise::new();
                futures.push(promise.future());
                let promise = Mutex::new(Some(promise));
                if let Some(pipeline) = self.pipeline.take() {
                    let sinkpad = pipeline
                        .by_name("display")
                        .unwrap()
                        .static_pad("sink")
                        .unwrap();
                    sinkpad.add_probe(gst::PadProbeType::EVENT_BOTH, move |_pad, info| match &info
                        .data
                    {
                        Some(gst::PadProbeData::Event(event)) => {
                            if let gst::EventView::Eos(_) = event.view() {
                                promise.lock().unwrap().take().unwrap().success(());
                                gst::PadProbeReturn::Remove
                            } else {
                                gst::PadProbeReturn::Pass
                            }
                        }
                        _ => gst::PadProbeReturn::Pass,
                    });
                    if pipeline.current_state() == gst::State::Playing
                        && pipeline.send_event(gst::event::Eos::new())
                    {
                        Future::sequence(futures.into_iter()).for_each(
                            clone!(@strong parent_sender, @weak pipeline => move |_| {
                                send!(parent_sender, SlaveMsg::PollingChanged(false));
                                pipeline.set_state(gst::State::Null).unwrap();
                            }),
                        );
                        glib::timeout_add_local_once(
                            Duration::from_secs(10),
                            clone!(@weak pipeline, @strong parent_sender => move || {
                                send!(parent_sender, SlaveMsg::PollingChanged(false));
                                if recording {
                                    send!(parent_sender, SlaveMsg::RecordingChanged(false));
                                }
                                send!(parent_sender, SlaveMsg::ShowToastMessage(String::from("等待管道响应超时，已将其强制终止。")));
                                pipeline.set_state(gst::State::Null).unwrap();
                            }),
                        );
                    } else {
                        send!(parent_sender, SlaveMsg::PollingChanged(false));
                        send!(parent_sender, SlaveMsg::RecordingChanged(false));
                        pipeline.set_state(gst::State::Null).unwrap();
                    }
                }
            }
            SlaveVideoMsg::SaveScreenshot(pathbuf) => {
                assert!(self.pixbuf != None);
                if let Some(pixbuf) = &self.pixbuf {
                    match pixbuf.savev(&pathbuf, "jpeg", &[]) {
                        Ok(_) => send!(
                            parent_sender,
                            SlaveMsg::ShowToastMessage(format!(
                                "截图保存成功：{}",
                                pathbuf.to_str().unwrap()
                            ))
                        ),
                        Err(err) => send!(
                            parent_sender,
                            SlaveMsg::ShowToastMessage(format!(
                                "截图保存失败：{}",
                                err.to_string()
                            ))
                        ),
                    }
                }
            }
            SlaveVideoMsg::RequestFrame => {
                if let Some(pipeline) = &self.pipeline {
                    pipeline
                        .by_name("display")
                        .unwrap()
                        .dynamic_cast::<gst_app::AppSink>()
                        .unwrap()
                        .send_event(gst::event::CustomDownstream::new(gst::Structure::new(
                            "resend",
                            &[],
                        )));
                }
            }
        }
    }
}

impl std::fmt::Debug for SlaveVideoWidgets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.root_widget().fmt(f)
    }
}

#[micro_widget(pub)]
impl MicroWidgets<SlaveVideoModel> for SlaveVideoWidgets {
    view! {
        frame = GtkBox {
            append = &Stack {
                set_vexpand: true,
                set_hexpand: true,
                add_child = &StatusPage {
                    set_icon_name: Some("mail-mark-junk-symbolic"),
                    set_title: "无信号",
                    set_description: Some("请点击上方按钮启动视频拉流"),
                    set_visible: track!(model.changed(SlaveVideoModel::pixbuf()), model.pixbuf == None),
                },
                add_child = &Picture {
                    set_hexpand: true,
                    set_vexpand: true,
                    set_can_shrink: true,
                    set_keep_aspect_ratio: track!(model.changed(SlaveVideoModel::config()), *model.config.lock().unwrap().get_keep_video_display_ratio()),
                    set_pixbuf: track!(model.changed(SlaveVideoModel::pixbuf()), match &model.pixbuf {
                        Some(pixbuf) => Some(&pixbuf),
                        None => None,
                    }),
                },
            },
        }
    }
}
