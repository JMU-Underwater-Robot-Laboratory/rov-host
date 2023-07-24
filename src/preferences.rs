/* preferences.rs
 *
 *   Copyright (C) 2021-2023  Bohong Huang, Jianfeng Peng, JMU Underwater Lab
 *
 *   This program is free software: you can redistribute it and/or modify
 *   it under the terms of the GNU General Public License as published by
 *   the Free Software Foundation, either version 3 of the License, or
 *   (at your option) any later version.
 *
 *   This program is distributed in the hope that it will be useful,
 *   but WITHOUT ANY WARRANTY; without even the implied warranty of
 *   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 *   GNU General Public License for more details.
 *
 *   You should have received a copy of the GNU General Public License
 *   along with this program.  If not, see <https://www.gnu.org/licenses/>.
 */

use std::{fs, path::PathBuf, str::FromStr};

use adw::{
    prelude::*, ActionRow, PreferencesGroup, PreferencesPage,
    PreferencesWindow,
};
use glib::Sender;
use gtk::{Align, Entry, Inhibit, Switch};
use relm4::{send, ComponentUpdate, Model, Widgets};
use relm4_macros::widget;

use derivative::*;
use serde::{Deserialize, Serialize};

use url::Url;

use crate::{
     AppModel, AppMsg,
};

pub fn get_data_path() -> PathBuf {
    const APP_DIR_NAME: &str = "rovhost";
    let mut data_path = dirs::data_local_dir().expect("无法找到本地数据文件夹");
    data_path.push(APP_DIR_NAME);
    if !data_path.exists() {
        fs::create_dir(data_path.clone()).expect("无法创建应用数据文件夹");
    }
    data_path
}

pub fn get_preference_path() -> PathBuf {
    let mut path = get_data_path();
    path.push("preferences.json");
    path
}

pub fn get_video_path() -> PathBuf {
    let mut video_path = get_data_path();
    video_path.push("Videos");
    if !video_path.exists() {
        fs::create_dir(video_path.clone()).expect("无法创建视频文件夹");
    }
    video_path
}

pub fn get_image_path() -> PathBuf {
    let mut video_path = get_data_path();
    video_path.push("Images");
    if !video_path.exists() {
        fs::create_dir(video_path.clone()).expect("无法创建图片文件夹");
    }
    video_path
}

#[tracker::track]
#[derive(Derivative, Clone, PartialEq, Debug, Serialize, Deserialize)]
#[derivative(Default)]
pub struct PreferencesModel {
    #[derivative(Default(value = "Url::from_str(\"http://192.168.137.219:8888\").unwrap()"))]
    pub default_slave_url: Url,
    #[derivative(Default(
        value = "Url::from_str(\"rtsp://rov:rov@192.168.137.123:554/\").unwrap()"
    ))]
    pub default_video_url: Url,
    #[derivative(Default(value = "true"))]
    pub default_keep_video_display_ratio: bool,
}

impl PreferencesModel {
    pub fn load_or_default() -> PreferencesModel {
        match fs::read_to_string(get_preference_path())
            .ok()
            .and_then(|json| serde_json::from_str(&json).ok())
        {
            Some(model) => model,
            None => Default::default(),
        }
    }
}

#[derive(Debug)]
pub enum PreferencesMsg {
    SetDefaultKeepVideoDisplayRatio(bool),
    SetDefaultVideoUrl(Url),
    SetDefaultSlaveUrl(Url),
    SaveToFile,
    OpenVideoDirectory,
    OpenImageDirectory,
}

impl Model for PreferencesModel {
    type Msg = PreferencesMsg;
    type Widgets = PreferencesWidgets;
    type Components = ();
}

#[widget(pub)]
impl Widgets<PreferencesModel, AppModel> for PreferencesWidgets {
    view! {
        window = PreferencesWindow {
            set_title: Some("首选项"),
            set_transient_for: parent!(Some(&parent_widgets.app_window)),
            set_destroy_with_parent: true,
            set_modal: true,
            set_search_enabled: false,
            connect_close_request(sender) => move |window| {
                send!(sender, PreferencesMsg::SaveToFile);
                window.hide();
                Inhibit(true)
            },
            add = &PreferencesPage {
                set_title: "通信",
                set_icon_name: Some("network-transmit-receive-symbolic"),
                add = &PreferencesGroup {
                    set_description: Some("与机器人的连接通信设置"),
                    set_title: "连接",
                    add = &ActionRow {
                        set_title: "默认连接 URL",
                        set_subtitle: "连接第一机位的机器人使用的默认 URL，其他机位会自动累加 IPV4 地址",
                        add_suffix = &Entry {
                            set_text: track!(model.changed(PreferencesModel::default_slave_url()), model.get_default_slave_url().to_string().as_str()),
                            set_valign: Align::Center,
                            set_width_request: 200,
                            connect_changed(sender) => move |entry| {
                                if let Ok(url) = Url::from_str(&entry.text()) {
                                    send!(sender, PreferencesMsg::SetDefaultSlaveUrl(url));
                                    entry.remove_css_class("error");
                                } else {
                                    entry.add_css_class("error");
                                }
                            }
                         },
                    },
                },
            },
            add = &PreferencesPage {
                set_title: "视频",
                set_icon_name: Some("video-display-symbolic"),
                add = &PreferencesGroup {
                    set_title: "显示",
                    set_description: Some("上位机的显示的画面设置"),
                    add = &ActionRow {
                        set_title: "默认保持长宽比",
                        set_subtitle: "在改变窗口大小的时是否保持画面比例，这可能导致画面无法全屏",
                        add_suffix: default_keep_video_display_ratio_switch = &Switch {
                            set_active: track!(model.changed(PreferencesModel::default_keep_video_display_ratio()), model.default_keep_video_display_ratio),
                            set_valign: Align::Center,
                            connect_state_set(sender) => move |_switch, state| {
                                send!(sender, PreferencesMsg::SetDefaultKeepVideoDisplayRatio(state));
                                Inhibit(false)
                            }
                        },
                        set_activatable_widget: Some(&default_keep_video_display_ratio_switch),
                    },
                },
                add = &PreferencesGroup {
                    set_title: "管道",
                    set_description: Some("配置拉流以及录制所使用的管道"),
                    add = &ActionRow {
                        set_title: "默认视频 URL",
                        set_subtitle: "第一机位使用的视频 URL，其他机位会自动累加端口",
                        add_suffix = &Entry {
                            set_text: track!(model.changed(PreferencesModel::default_video_url()), model.get_default_video_url().to_string().as_str()),
                            set_valign: Align::Center,
                            set_width_request: 200,
                            connect_changed(sender) => move |entry| {
                                if let Ok(url) = Url::from_str(&entry.text()) {
                                    send!(sender, PreferencesMsg::SetDefaultVideoUrl(url));
                                    entry.remove_css_class("error");
                                } else {
                                    entry.add_css_class("error");
                                }
                            }
                        },
                    },
                },
                add = &PreferencesGroup {
                    set_title: "截图",
                    set_description: Some("画面的截图选项"),
                    add = &ActionRow {
                        set_title: "图片保存目录",
                        set_subtitle: crate::preferences::get_image_path().to_str().unwrap(),
                        set_activatable: true,
                        connect_activated(sender) => move |_row| {
                            send!(sender, PreferencesMsg::OpenImageDirectory);
                        }
                    },
                },
                add = &PreferencesGroup {
                    set_title: "录制",
                    set_description: Some("视频流的录制选项"),
                    add = &ActionRow {
                        set_title: "视频保存目录",
                        set_subtitle: crate::preferences::get_video_path().to_str().unwrap(),
                        set_activatable: true,
                        connect_activated(sender) => move |_row| {
                            send!(sender, PreferencesMsg::OpenVideoDirectory);
                        }
                    },
                },
            },
        }
    }

    fn post_init() {}
}

impl ComponentUpdate<AppModel> for PreferencesModel {
    fn init_model(parent_model: &AppModel) -> Self {
        parent_model.preferences.borrow().clone()
    }

    fn update(
        &mut self,
        msg: PreferencesMsg,
        _components: &(),
        _sender: Sender<PreferencesMsg>,
        parent_sender: Sender<AppMsg>,
    ) {
        self.reset();
        match msg {
            PreferencesMsg::SetDefaultKeepVideoDisplayRatio(value) => {
                self.set_default_keep_video_display_ratio(value) // 设置默认的视频显示比例
            }
            
            PreferencesMsg::SaveToFile => serde_json::to_string_pretty(&self) // 将当前对象序列为 JSON 字符串
                .ok()
                .and_then(|json| fs::write(get_preference_path(), json).ok()) // 将 JSON 字符串写文件
                .unwrap(),
            
            PreferencesMsg::OpenVideoDirectory => gtk::show_uri( // 打开视频目录
             None as Option<&PreferencesWindow>,
                glib::filename_to_uri(crate::preferences::get_video_path().to_str().unwrap(), None) // 获取视频路径并转换为 URI
                    .unwrap()
                    .as_str(),
                gdk::CURRENT_TIME,
            ),
            
            PreferencesMsg::OpenImageDirectory => gtk::show_uri( // 打开图目录
                None as Option<&PreferencesWindow>,
                glib::filename_to_uri(crate::preferences::get_video_path().to_str().unwrap(), None) // 获取图像路径并转换为 URI
                    .unwrap()
                    .as_str(),
                gdk::CURRENT_TIME,
            ),
            
            PreferencesMsg::SetDefaultVideoUrl(url) => self.default_video_url = url, // 设置默认的视频 URL（防输入框光标移动至最前）
            
            PreferencesMsg::SetDefaultSlaveUrl(url) => self.default_slave_url = url, // 设置默认的从属 URL
        }
        send!(parent_sender, AppMsg::PreferencesUpdated(self.clone()));
    }
}
