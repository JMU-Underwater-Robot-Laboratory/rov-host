/* slave_config.rs
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

use std::{fmt::Debug, str::FromStr};

use adw::{prelude::*, ActionRow, ComboRow, ExpanderRow, PreferencesGroup};
use glib::Sender;
use gtk::{
    Align, Box as GtkBox, Entry, Inhibit, Label, Orientation, ScrolledWindow, Separator,
    SpinButton, StringList, Switch, Viewport,
};
use relm4::{send, MicroModel, MicroWidgets, WidgetPlus};
use relm4_macros::micro_widget;

use derivative::*;
use strum::IntoEnumIterator;
use url::Url;

use super::{
    video::{VideoAlgorithm, VideoEncoder},
    SlaveMsg,
};
use crate::{
    preferences::PreferencesModel,
    slave::video::{ColorspaceConversion, VideoCodec, VideoCodecProvider, VideoDecoder},
};

#[tracker::track]
#[derive(Debug, Derivative, PartialEq, Clone)]
#[derivative(Default)]
pub struct SlaveConfigModel {
    #[derivative(Default(value = "Some(false)"))]
    polling: Option<bool>,
    #[derivative(Default(value = "Some(false)"))]
    connected: Option<bool>,
    #[derivative(Default(value = "PreferencesModel::default().default_slave_url"))]
    pub slave_url: Url,
    #[derivative(Default(value = "PreferencesModel::default().default_video_url"))]
    pub video_url: Url,
    pub video_algorithms: Vec<VideoAlgorithm>,
    #[derivative(Default(value = "PreferencesModel::default().default_keep_video_display_ratio"))]
    pub keep_video_display_ratio: bool,
    #[derivative(Default(value = "PreferencesModel::default().default_video_decoder"))]
    pub video_decoder: VideoDecoder,
    #[derivative(Default(value = "PreferencesModel::default().default_colorspace_conversion"))]
    pub colorspace_conversion: ColorspaceConversion,
    #[derivative(Default(value = "false"))]
    pub swap_xy: bool,
    #[derivative(Default(value = "PreferencesModel::default().default_use_decodebin"))]
    pub use_decodebin: bool,
    pub video_encoder: VideoEncoder,
    pub reencode_recording_video: bool,
    #[derivative(Default(
        value = "PreferencesModel::default().default_appsink_queue_leaky_enabled"
    ))]
    pub appsink_queue_leaky_enabled: bool,
    #[derivative(Default(value = "PreferencesModel::default().default_video_latency"))]
    pub video_latency: u32,
}

impl SlaveConfigModel {
    pub fn from_preferences(preferences: &PreferencesModel) -> Self {
        Self {
            slave_url: preferences.get_default_slave_url().clone(),
            video_url: preferences.get_default_video_url().clone(),
            colorspace_conversion: preferences.get_default_colorspace_conversion().clone(),
            video_decoder: preferences.get_default_video_decoder().clone(),
            keep_video_display_ratio: preferences.get_default_keep_video_display_ratio().clone(),
            use_decodebin: preferences.get_default_use_decodebin().clone(),
            video_encoder: preferences.get_default_video_encoder().clone(),
            reencode_recording_video: preferences.get_default_reencode_recording_video().clone(),
            appsink_queue_leaky_enabled: preferences
                .get_default_appsink_queue_leaky_enabled()
                .clone(),
            video_latency: preferences.get_default_video_latency().clone(),
            ..Default::default()
        }
    }
}

impl MicroModel for SlaveConfigModel {
    type Msg = SlaveConfigMsg;
    type Widgets = SlaveConfigWidgets;
    type Data = Sender<SlaveMsg>;
    fn update(
        &mut self,
        msg: SlaveConfigMsg,
        parent_sender: &Sender<SlaveMsg>,
        _sender: Sender<SlaveConfigMsg>,
    ) {
        self.reset();
        match msg {
            SlaveConfigMsg::SetKeepVideoDisplayRatio(value) => {
                self.set_keep_video_display_ratio(value)
            }
            SlaveConfigMsg::SetPolling(polling) => self.set_polling(polling),
            SlaveConfigMsg::SetConnected(connected) => self.set_connected(connected),
            SlaveConfigMsg::SetVideoAlgorithm(algorithm) => {
                self.get_mut_video_algorithms().clear();
                if let Some(algorithm) = algorithm {
                    self.get_mut_video_algorithms().push(algorithm);
                }
            }
            SlaveConfigMsg::SetVideoDecoder(decoder) => self.set_video_decoder(decoder),
            SlaveConfigMsg::SetColorspaceConversion(conversion) => {
                self.set_colorspace_conversion(conversion)
            }
            SlaveConfigMsg::SetVideoUrl(url) => self.video_url = url,
            SlaveConfigMsg::SetSlaveUrl(url) => self.slave_url = url,
            SlaveConfigMsg::SetVideoDecoderCodec(codec) => self.get_mut_video_decoder().0 = codec,
            SlaveConfigMsg::SetVideoDecoderCodecProvider(provider) => {
                self.get_mut_video_decoder().1 = provider
            }
            SlaveConfigMsg::SetSwapXY(swap) => self.set_swap_xy(swap),
            SlaveConfigMsg::SetUsePlaybin(use_decodebin) => {
                if use_decodebin {
                    self.set_reencode_recording_video(true);
                }
                self.set_use_decodebin(use_decodebin);
            }
            SlaveConfigMsg::SetVideoEncoderCodec(codec) => self.get_mut_video_encoder().0 = codec,
            SlaveConfigMsg::SetVideoEncoderCodecProvider(provider) => {
                self.get_mut_video_encoder().1 = provider
            }
            SlaveConfigMsg::SetReencodeRecordingVideo(reencode) => {
                if !reencode {
                    self.set_use_decodebin(false);
                }
                self.set_reencode_recording_video(reencode)
            }
            SlaveConfigMsg::SetAppSinkQueueLeakyEnabled(leaky) => {
                self.set_appsink_queue_leaky_enabled(leaky)
            }
            SlaveConfigMsg::SetVideoLatency(latency) => self.set_video_latency(latency),
        }
        send!(parent_sender, SlaveMsg::ConfigUpdated);
    }
}

impl std::fmt::Debug for SlaveConfigWidgets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.root_widget().fmt(f)
    }
}

pub enum SlaveConfigMsg {
    SetVideoUrl(Url),
    SetSlaveUrl(Url),
    SetKeepVideoDisplayRatio(bool),
    SetPolling(Option<bool>),
    SetConnected(Option<bool>),
    SetVideoAlgorithm(Option<VideoAlgorithm>),
    SetVideoDecoder(VideoDecoder),
    SetColorspaceConversion(ColorspaceConversion),
    SetVideoDecoderCodec(VideoCodec),
    SetVideoDecoderCodecProvider(VideoCodecProvider),
    SetSwapXY(bool),
    SetUsePlaybin(bool),
    SetVideoEncoderCodec(VideoCodec),
    SetVideoEncoderCodecProvider(VideoCodecProvider),
    SetReencodeRecordingVideo(bool),
    SetAppSinkQueueLeakyEnabled(bool),
    SetVideoLatency(u32),
}

#[micro_widget(pub)]
impl MicroWidgets<SlaveConfigModel> for SlaveConfigWidgets {
    view! {
        window = GtkBox {
            add_css_class: "background",
            set_orientation: Orientation::Horizontal,
            set_hexpand: false,
            append = &Separator {
                set_orientation: Orientation::Horizontal,
            },
            append = &ScrolledWindow {
                set_width_request: 340,
                set_child = Some(&Viewport) {
                    set_child = Some(&GtkBox) {
                        set_spacing: 20,
                        set_margin_all: 10,
                        set_orientation: Orientation::Vertical,
                        append = &PreferencesGroup {
                            set_sensitive: track!(model.changed(SlaveConfigModel::connected()), model.get_connected().eq(&Some(false))),
                            set_title: "通讯",
                            set_description: Some("设置下位机的通讯选项"),
                            add = &ActionRow {
                                set_title: "连接 URL",
                                set_subtitle: "连接下位机使用的 URL",
                                add_suffix = &Entry {
                                    set_text: model.get_slave_url().to_string().as_str(),
                                    set_width_request: 160,
                                    set_valign: Align::Center,
                                    connect_changed(sender) => move |entry| {
                                        if let Ok(url) = Url::from_str(&entry.text()) {
                                            send!(sender, SlaveConfigMsg::SetSlaveUrl(url));
                                            entry.remove_css_class("error");
                                        } else {
                                            entry.add_css_class("error");
                                        }
                                    }
                                },
                            },
                        },
                        append = &PreferencesGroup {
                            set_title: "画面",
                            set_description: Some("上位机端对画面进行的处理选项"),

                            add = &ActionRow {
                                set_title: "保持长宽比",
                                set_subtitle: "在改变窗口大小的时是否保持画面比例，这可能导致画面无法全屏",
                                add_suffix: default_keep_video_display_ratio_switch = &Switch {
                                    set_active: track!(model.changed(SlaveConfigModel::keep_video_display_ratio()), *model.get_keep_video_display_ratio()),
                                    set_valign: Align::Center,
                                    connect_state_set(sender) => move |_switch, state| {
                                        send!(sender, SlaveConfigMsg::SetKeepVideoDisplayRatio(state));
                                        Inhibit(false)
                                    }
                                },
                                set_activatable_widget: Some(&default_keep_video_display_ratio_switch),
                            },
                            add = &ComboRow {
                                set_title: "增强算法",
                                set_subtitle: "对画面使用的增强算法",
                                set_model: Some(&{
                                    let model = StringList::new(&[]);
                                    model.append("无");
                                    for value in VideoAlgorithm::iter() {
                                        model.append(&value.to_string());
                                    }
                                    model
                                }),
                                set_selected: track!(model.changed(SlaveConfigModel::video_algorithms()), VideoAlgorithm::iter().position(|x| model.video_algorithms.first().map_or_else(|| false, |y| *y == x)).map_or_else(|| 0, |x| x + 1) as u32),
                                connect_selected_notify(sender) => move |row| {
                                    send!(sender, SlaveConfigMsg::SetVideoAlgorithm(if row.selected() > 0 { Some(VideoAlgorithm::iter().nth(row.selected().wrapping_sub(1) as usize).unwrap()) } else { None }));
                                }
                            }
                        },
                        append = &PreferencesGroup {
                            set_sensitive: track!(model.changed(SlaveConfigModel::polling()), model.get_polling().eq(&Some(false))),
                            set_title: "管道",
                            set_description: Some("配置视频流接收以及录制所使用的管道"),
                            add = &ActionRow {
                                set_title: "视频流 URL",
                                set_subtitle: "配置机位视频流的 URL",
                                add_suffix = &Entry {
                                    set_text: track!(model.changed(SlaveConfigModel::video_url()), model.get_video_url().to_string().as_str()),
                                    set_valign: Align::Center,
                                    set_width_request: 160,
                                    connect_changed(sender) => move |entry| {
                                        if let Ok(url) = Url::from_str(&entry.text()) {
                                            send!(sender, SlaveConfigMsg::SetVideoUrl(url));
                                            entry.remove_css_class("error");
                                        } else {
                                            entry.add_css_class("error");
                                        }
                                    }
                                },
                            },
                            add = &ExpanderRow {
                                set_title: "手动配置管道",
                                set_show_enable_switch: true,
                                set_expanded: !*model.get_use_decodebin(),
                                set_enable_expansion: track!(model.changed(SlaveConfigModel::use_decodebin()), !*model.get_use_decodebin()),
                                connect_enable_expansion_notify(sender) => move |expander| {
                                    send!(sender, SlaveConfigMsg::SetUsePlaybin(!expander.enables_expansion()));
                                },
                                add_row = &ActionRow {
                                    set_title: "接收缓冲区延迟",
                                    set_subtitle: "可以增加接收缓冲区延迟，牺牲视频的实时性来换取流畅度的提升",
                                    add_suffix = &SpinButton::with_range(0.0, 60000.0, 50.0) {
                                        set_value: track!(model.changed(SlaveConfigModel::video_latency()), model.video_latency as f64),
                                        set_digits: 0,
                                        set_valign: Align::Center,
                                        set_can_focus: false,
                                        connect_value_changed(sender) => move |button| {
                                            send!(sender, SlaveConfigMsg::SetVideoLatency(button.value() as u32));
                                        }
                                    },
                                    add_suffix = &Label {
                                        set_label: "毫秒",
                                    },
                                },
                            },
                            add = &ExpanderRow {
                                set_title: "录制时重新编码",
                                set_show_enable_switch: true,
                                set_expanded: *model.get_reencode_recording_video(),
                                set_enable_expansion: track!(model.changed(SlaveConfigModel::reencode_recording_video()), *model.get_reencode_recording_video()),
                                connect_enable_expansion_notify(sender) => move |expander| {
                                    send!(sender, SlaveConfigMsg::SetReencodeRecordingVideo(expander.enables_expansion()));
                                },
                                add_row = &ComboRow {
                                    set_title: "编码器",
                                    set_subtitle: "视频录制时使用的编码器",
                                    set_model: Some(&{
                                        let model = StringList::new(&[]);
                                        for value in VideoCodec::iter() {
                                            model.append(&value.to_string());
                                        }
                                        model
                                    }),
                                    set_selected: track!(model.changed(SlaveConfigModel::video_encoder()), VideoCodec::iter().position(|x| x == model.video_encoder.0).unwrap() as u32),
                                    connect_selected_notify(sender) => move |row| {
                                        send!(sender, SlaveConfigMsg::SetVideoEncoderCodec(VideoCodec::iter().nth(row.selected() as usize).unwrap()))
                                    }
                                },
                            },
                        },
                    },
                },
            },
        }
    }
}
