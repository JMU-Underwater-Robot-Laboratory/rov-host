/* mod.rs
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

pub mod firmware_update;
pub mod param_tuner;
pub mod protocol;
pub mod slave_config;
pub mod slave_video;
pub mod video;

use async_std::task::{self, JoinHandle};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    error::Error,
    fmt::Debug,
    ops::Deref,
    rc::Rc,
    sync::{Arc, Mutex},
    time::{Duration, SystemTime},
};

use adw::{ApplicationWindow, Flap, FlapFoldPolicy, Toast, ToastOverlay};
use glib::{DateTime, MainContext, Sender, WeakRef, PRIORITY_DEFAULT};
use glib_macros::clone;
use gtk::{
    prelude::*, Align, Box as GtkBox, Button as GtkButton, CenterBox, CheckButton, Frame, Grid,
    Image, Inhibit, Label, ListBox, MenuButton, Orientation, Overlay, PackType, Popover, Revealer,
    Separator, Switch, ToggleButton, Widget,
};
use relm4::{
    factory::{positions::GridPosition, FactoryPrototype, FactoryVec},
    send, MicroComponent, MicroModel, MicroWidgets, WidgetPlus,
};
use relm4_macros::micro_widget;

use jsonrpsee_core::{client::ClientT, Error as RpcError};
use jsonrpsee_http_client::{HttpClient, HttpClientBuilder};

use derivative::*;
use serde::{Deserialize, Serialize};

use self::{
    firmware_update::SlaveFirmwareUpdaterModel,
    param_tuner::SlaveParameterTunerModel,
    protocol::*,
    slave_config::{SlaveConfigModel, SlaveConfigMsg},
    slave_video::{SlaveVideoModel, SlaveVideoMsg},
};
use crate::preferences::PreferencesModel;
use crate::ui::generic::error_message;
use crate::AppMsg;
use crate::{
    input::{Axis, Button, InputSource, InputSourceEvent, InputSystem},
    slave::param_tuner::SlaveParameterTunerMsg,
};

pub type RpcClient = HttpClient;
pub type RpcClientBuilder = HttpClientBuilder;
pub type RpcParams = jsonrpsee_http_client::types::ParamsSer<'static>;

#[tracker::track]
#[derive(Debug, Derivative)]
#[derivative(Default)]
pub struct SlaveModel {
    #[no_eq]
    #[derivative(Default(
        value = "MyComponent::new(Default::default(), MainContext::channel(PRIORITY_DEFAULT).0)"
    ))]
    pub config: MyComponent<SlaveConfigModel>,
    #[no_eq]
    #[derivative(Default(
        value = "MyComponent::new(Default::default(), MainContext::channel(PRIORITY_DEFAULT).0)"
    ))]
    pub video: MyComponent<SlaveVideoModel>,
    #[derivative(Default(value = "Some(false)"))]
    pub connected: Option<bool>,
    #[derivative(Default(value = "Some(false)"))]
    pub polling: Option<bool>,
    #[derivative(Default(value = "Some(false)"))]
    pub recording: Option<bool>,
    pub sync_recording: bool,
    #[no_eq]
    pub preferences: Rc<RefCell<PreferencesModel>>,
    pub input_sources: HashSet<InputSource>,
    #[no_eq]
    pub input_system: Rc<InputSystem>,
    #[no_eq]
    #[derivative(Default(value = "MainContext::channel(PRIORITY_DEFAULT).0"))]
    pub input_event_sender: Sender<InputSourceEvent>,
    #[derivative(Default(value = "true"))]
    pub slave_info_displayed: bool,
    #[no_eq]
    pub status: Arc<Mutex<HashMap<SlaveStatusClass, i16>>>,
    #[no_eq]
    pub communication_msg_sender: Option<async_std::channel::Sender<SlaveCommunicationMsg>>,
    #[no_eq]
    pub rpc_client: Option<async_std::sync::Arc<RpcClient>>,
    pub toast_messages: Rc<RefCell<VecDeque<String>>>,
    #[no_eq]
    #[derivative(Default(value = "FactoryVec::new()"))]
    pub infos: FactoryVec<SlaveInfoModel>,
    pub config_presented: bool,
}

#[tracker::track]
#[derive(Debug, Derivative)]
#[derivative(Default)]
pub struct SlaveInfoModel {
    key: String,
    value: String,
}

#[relm4::factory_prototype(pub)]
impl FactoryPrototype for SlaveInfoModel {
    type Factory = FactoryVec<Self>;
    type Widgets = SlaveInfoWidgets;
    type View = GtkBox;
    type Msg = SlaveMsg;

    view! {
        entry = CenterBox {
            set_orientation: Orientation::Horizontal,
            set_hexpand: true,
            set_start_widget = Some(&Label) {
                set_valign: Align::Start,
                set_markup: track!(self.changed(SlaveInfoModel::key()), &format!("<b>{}</b>", self.get_key())),
            },
            set_end_widget = Some(&Label) {
                set_valign: Align::Start,
                set_label: track!(self.changed(SlaveInfoModel::value()), self.get_value()),
            }
        }
    }

    fn position(&self, _index: &usize) {}
}

#[derive(Debug, Hash, PartialEq, Eq, Clone)]
pub enum SlaveStatusClass {
    MotionX,
    MotionY,
    MotionZ,
    MotionRotate,
    RoboticArmOpen,
    RoboticArmClose,
    LightOpen,
    LightClose,
    DepthLocked,
    DirectionLocked,
}

impl SlaveStatusClass {
    pub fn from_button(button: Button) -> Option<SlaveStatusClass> {
        match button {
            Button::LeftStick => Some(SlaveStatusClass::DepthLocked),
            Button::RightStick => Some(SlaveStatusClass::DirectionLocked),
            Button::RightShoulder => Some(SlaveStatusClass::RoboticArmOpen),
            Button::LeftShoulder => Some(SlaveStatusClass::LightOpen),
            _ => None,
        }
    }

    pub fn from_axis(axis: Axis) -> Option<SlaveStatusClass> {
        match axis {
            Axis::LeftX => Some(SlaveStatusClass::MotionX),
            Axis::LeftY => Some(SlaveStatusClass::MotionY),
            Axis::RightX => Some(SlaveStatusClass::MotionRotate),
            Axis::RightY => Some(SlaveStatusClass::MotionZ),
            Axis::TriggerRight => Some(SlaveStatusClass::RoboticArmClose),
            Axis::TriggerLeft => Some(SlaveStatusClass::LightClose),
        }
    }
}

const JOYSTICK_DISPLAY_THRESHOLD: i16 = 500;

impl SlaveModel {
    pub fn new(
        config: SlaveConfigModel,
        preferences: Rc<RefCell<PreferencesModel>>,
        component_sender: &Sender<SlaveMsg>,
        input_event_sender: Sender<InputSourceEvent>,
    ) -> Self {
        Self {
            config: MyComponent::new(config.clone(), component_sender.clone()),
            video: MyComponent::new(
                SlaveVideoModel::new(preferences.clone(), Arc::new(Mutex::new(config))),
                component_sender.clone(),
            ),
            preferences,
            input_event_sender,
            status: Arc::new(Mutex::new(HashMap::new())),
            ..Default::default()
        }
    }

    pub fn get_target_status_or_insert_0(&mut self, status_class: &SlaveStatusClass) -> i16 {
        let mut status = self.status.lock().unwrap();
        *status.entry(status_class.clone()).or_insert(0)
    }

    pub fn get_target_status(&self, status_class: &SlaveStatusClass) -> i16 {
        let status = self.status.lock().unwrap();
        *status.get(status_class).unwrap_or(&0)
    }
    pub fn set_target_status(&mut self, status_class: &SlaveStatusClass, new_status: i16) {
        let mut status = self.get_mut_status().lock().unwrap();
        *status.entry(status_class.clone()).or_insert(0) = new_status;
    }
}

pub fn input_sources_list_box(
    input_sources: &HashSet<InputSource>,
    input_system: &InputSystem,
    sender: &Sender<SlaveMsg>,
) -> Widget {
    let sources = input_system.get_sources().unwrap();
    // 获取输入系统的设备列表

    if sources.is_empty() {
        return Label::builder()
            .label("无可用设备")
            .margin_top(4)
            .margin_bottom(4)
            .margin_start(4)
            .margin_end(4)
            .build()
            .upcast();
    }
    // 如果设备列表为空，则返回一个带有指定标签和边距的标签控件

    let list_box = ListBox::builder().build();
    // 创建一个列表控件

    let mut radio_button_group: Option<CheckButton> = None;
    // 创建一个可选的单选按钮

    for (source, name) in sources {
        // 遍历设备列表中的每个设备及其名称

        let radio_button = CheckButton::builder().label(&name).build();
        // 创建一个带有指定名称的单选按钮

        let sender = sender.clone();
        // 克隆发送器

        radio_button.set_active(input_sources.contains(&source));
        // 设置单选按钮的活动状态，如果输入源包含当前设备，则设置为活动状态

        radio_button.connect_toggled(move |button| {
            // 监听单选按钮的切换事件
            if button.is_active() {
                send!(sender, SlaveMsg::AddInputSource(source.clone()));
                // 如果单选按钮被激活，则向发送器发送添加输入源的消息
            } else {
                send!(sender, SlaveMsg::RemoveInputSource(source.clone()));
                // 否，向发送器发送移除输入源的消息
            }
        });

        {
            let radio_button = radio_button.clone();
            // 克隆单选按钮

            match &radio_button_group {
                Some(button) => radio_button.set_group(Some(button)),
                // 如果单选按钮组存在，则将当前单选按钮设置为同一组
                None => radio_button_group = Some(radio_button),
                // 否则，将当前单选按钮设置为的单选按钮组
            }
        }

        list_box.append(&radio_button);
        // 将单选按钮添加到列表框中
    }

    list_box.upcast()
    // 返回列表框控件
}

#[micro_widget(pub)]
impl MicroWidgets<SlaveModel> for SlaveWidgets {
    view! {
        toast_overlay = ToastOverlay {
            add_toast?: watch!(model.get_toast_messages().borrow_mut().pop_front().map(|x| Toast::new(&x)).as_ref()),
            set_child = Some(&GtkBox) {
                set_orientation: Orientation::Vertical,
                append = &CenterBox {
                    set_css_classes: &["toolbar"],
                    set_orientation: Orientation::Horizontal,
                    set_start_widget = Some(&GtkBox) {
                        set_hexpand: true,
                        set_halign: Align::Start,
                        set_spacing: 5,
                        append = &ToggleButton {
                            set_icon_name: "emblem-system-symbolic",
                            set_css_classes: &["circular"],
                            set_tooltip_text: Some("机位设置"),
                            set_active: track!(model.changed(SlaveModel::config_presented()), *model.get_config_presented()),
                            connect_active_notify(sender) => move |button| {
                                send!(sender, SlaveMsg::SetConfigPresented(button.is_active()));
                            },
                        },
                        append = &Separator {},
                        append = &GtkButton {
                            set_icon_name: "preferences-other-symbolic",
                            set_css_classes: &["circular"],
                            set_tooltip_text: Some("参数调校"),
                            connect_clicked(sender) => move |_button| {
                                send!(sender, SlaveMsg::OpenParameterTuner);
                            },
                        },
                        append = &GtkButton {
                            set_icon_name: "software-update-available-symbolic",
                            set_css_classes: &["circular"],
                            set_tooltip_text: Some("固件更新"),
                            connect_clicked(sender) => move |_button| {
                                send!(sender, SlaveMsg::OpenFirmwareUpater);
                            },
                        },
                    },
                    set_center_widget = Some(&GtkBox) {
                        set_hexpand: true,
                        set_halign: Align::Center,
                        set_spacing: 5,
                        append = &Label {
                            set_text: track!(model.changed(SlaveModel::config()), model.config.model().get_slave_url().to_string().as_str()),
                        },
                        append = &MenuButton {
                            set_icon_name: "input-gaming-symbolic",
                            set_css_classes: &["circular"],
                            set_tooltip_text: Some("切换当前机位使用的输入设备"),
                            set_popover = Some(&Popover) {
                                set_child = Some(&GtkBox) {
                                    set_spacing: 5,
                                    set_orientation: Orientation::Vertical,
                                    append = &CenterBox {
                                        set_center_widget = Some(&Label) {
                                            set_margin_start: 10,
                                            set_margin_end: 10,
                                            set_markup: "<b>输入设备</b>"
                                        },
                                        set_end_widget = Some(&GtkButton) {
                                            set_icon_name: "view-refresh-symbolic",
                                            set_css_classes: &["circular"],
                                            set_tooltip_text: Some("刷新输入设备"),
                                            connect_clicked(sender) => move |_button| {
                                                send!(sender, SlaveMsg::UpdateInputSources);
                                            },
                                        },
                                    },
                                    append = &Frame {
                                        set_child: track!(model.changed(SlaveModel::input_system()), Some(&input_sources_list_box(&model.input_sources, &model.input_system ,&sender))),
                                    },

                                },
                            },
                        },
                    },
                    set_end_widget = Some(&GtkBox) {
                        set_hexpand: true,
                        set_halign: Align::End,
                        set_spacing: 5,
                        set_margin_end: 5,
                        append = &GtkButton {
                            set_icon_name: "camera-photo-symbolic",
                            set_sensitive: watch!(model.video.model().get_pixbuf().is_some()),
                            set_css_classes: &["circular"],
                            set_tooltip_text: Some("画面截图"),
                            connect_clicked(sender) => move |_button| {
                                send!(sender, SlaveMsg::TakeScreenshot);
                            },
                        },
                        append = &GtkButton {
                            set_icon_name: "camera-video-symbolic",
                            set_sensitive: track!(model.changed(SlaveModel::sync_recording()) || model.changed(SlaveModel::polling()) || model.changed(SlaveModel::recording()), !model.sync_recording && model.recording != None &&  model.polling == Some(true)),
                            set_css_classes?: watch!(model.recording.map(|x| if x { vec!["circular", "destructive-action"] } else { vec!["circular"] }).as_ref()),
                            set_tooltip_text: track!(model.changed(SlaveModel::recording()), model.recording.map(|x| if x { "停止录制" } else { "开始录制" })),
                            connect_clicked(sender) => move |_button| {
                                send!(sender, SlaveMsg::ToggleRecord);
                            },
                        },
                        append = &Separator {},
                        append = &GtkButton {
                            set_icon_name: "video-display-symbolic",
                            set_sensitive: track!(model.changed(SlaveModel::recording()) || model.changed(SlaveModel::sync_recording()) || model.changed(SlaveModel::polling()), model.get_recording().is_some() && model.get_polling().is_some() && !model.sync_recording),
                            set_css_classes?: watch!(model.polling.map(|x| if x { vec!["circular", "destructive-action"] } else { vec!["circular"] }).as_ref()),
                            set_tooltip_text: track!(model.changed(SlaveModel::polling()), model.polling.map(|x| if x { "停止拉流" } else { "启动拉流" })),
                            connect_clicked(sender) => move |_button| {
                                send!(sender, SlaveMsg::TogglePolling);
                            },
                        },
                        append = &GtkButton {
                            set_icon_name: "network-transmit-symbolic",
                            set_sensitive: track!(model.changed(SlaveModel::connected()), model.connected != None),
                            set_css_classes?: watch!(model.connected.map(|x| if x { vec!["circular", "suggested-action"] } else { vec!["circular"] }).as_ref()),
                            set_tooltip_text: track!(model.changed(SlaveModel::connected()), model.connected.map(|x| if x { "断开连接" } else { "连接" })),
                            connect_clicked(sender) => move |_button| {
                                send!(sender, SlaveMsg::ToggleConnect);
                            },
                        },
                    },
                },
                append = &Flap {
                    set_flap: Some(model.config.root_widget()),
                    set_reveal_flap: track!(model.changed(SlaveModel::config_presented()), *model.get_config_presented()),
                    set_fold_policy: FlapFoldPolicy::Auto,
                    set_locked: true,
                    set_flap_position: PackType::Start,
                    set_separator = Some(&Separator) {},
                    set_content = Some(&Overlay) {
                        set_width_request: 640,
                        set_child: Some(model.video.root_widget()),
                        add_overlay = &GtkBox {
                            set_valign: Align::Start,
                            set_halign: Align::Start,
                            set_hexpand: true,
                            set_margin_all: 20,
                            append = &Frame {
                                add_css_class: "card",
                                set_child = Some(&GtkBox) {
                                    set_orientation: Orientation::Vertical,
                                    set_margin_all: 5,
                                    set_width_request: 50,
                                    set_spacing: 5,
                                    append = &GtkButton {
                                        set_child = Some(&CenterBox) {
                                            set_center_widget = Some(&Label) {
                                                set_margin_start: 10,
                                                set_margin_end: 10,
                                                set_text: "状态信息",
                                            },
                                            set_end_widget = Some(&Image) {
                                                set_icon_name: watch!(Some(if model.slave_info_displayed { "go-down-symbolic" } else { "go-next-symbolic" })),
                                            },
                                        },
                                        connect_clicked(sender) => move |_button| {
                                            send!(sender, SlaveMsg::ToggleDisplayInfo);
                                        },
                                    },
                                    append = &Revealer {
                                        set_reveal_child: watch!(model.slave_info_displayed),
                                        set_child = Some(&GtkBox) {
                                            set_spacing: 5,
                                            set_margin_all: 5,
                                            set_orientation: Orientation::Vertical,
                                            set_halign: Align::Center,
                                            append = &GtkBox {
                                                set_hexpand: true,
                                                set_halign: Align::Center,
                                                append = &Grid {
                                                    set_margin_all: 2,
                                                    set_row_spacing: 2,
                                                    set_column_spacing: 2,
                                                    attach(0, 0, 1, 1) = &ToggleButton {
                                                        set_icon_name: "go-last-symbolic",
                                                        set_can_focus: false,
                                                        set_can_target: false,
                                                        set_active: track!(model.changed(SlaveModel::status()), model.get_target_status(&SlaveStatusClass::RoboticArmClose) > 0),
                                                    },
                                                    attach(1, 0, 1, 1) = &ToggleButton {
                                                        set_icon_name: "object-flip-horizontal-symbolic",
                                                        set_can_focus: false,
                                                        set_can_target: false,
                                                        set_active: track!(model.changed(SlaveModel::status()), model.get_target_status(&SlaveStatusClass::RoboticArmOpen) > 0),
                                                    },
                                                    attach(2, 0, 1, 1) = &ToggleButton {
                                                        set_icon_name: "go-first-symbolic",
                                                        set_can_focus: false,
                                                        set_can_target: false,
                                                        set_active: track!(model.changed(SlaveModel::status()), model.get_target_status(&SlaveStatusClass::RoboticArmClose) > 0),
                                                    },
                                                    attach(0, 1, 1, 1) = &ToggleButton {
                                                        set_icon_name: "object-rotate-left-symbolic",
                                                        set_can_focus: false,
                                                        set_can_target: false,
                                                        set_active: track!(model.changed(SlaveModel::status()), model.get_target_status(&SlaveStatusClass::MotionRotate) < -JOYSTICK_DISPLAY_THRESHOLD),
                                                    },
                                                    attach(2, 1, 1, 1) = &ToggleButton {
                                                        set_icon_name: "object-rotate-right-symbolic",
                                                        set_can_focus: false,
                                                        set_can_target: false,
                                                        set_active: track!(model.changed(SlaveModel::status()), model.get_target_status(&SlaveStatusClass::MotionRotate) > JOYSTICK_DISPLAY_THRESHOLD),
                                                    },
                                                    attach(0, 3, 1, 1) = &ToggleButton {
                                                        set_icon_name: "go-bottom-symbolic",
                                                        set_can_focus: false,
                                                        set_can_target: false,
                                                        set_active: track!(model.changed(SlaveModel::status()), model.get_target_status(&SlaveStatusClass::MotionZ) < -JOYSTICK_DISPLAY_THRESHOLD),
                                                    },
                                                    attach(2, 3, 1, 1) = &ToggleButton {
                                                        set_icon_name: "go-top-symbolic",
                                                        set_can_focus: false,
                                                        set_can_target: false,
                                                        set_active: track!(model.changed(SlaveModel::status()), model.get_target_status(&SlaveStatusClass::MotionZ) > JOYSTICK_DISPLAY_THRESHOLD),
                                                    },
                                                    attach(1, 1, 1, 1) = &ToggleButton {
                                                        set_icon_name: "go-up-symbolic",
                                                        set_can_focus: false,
                                                        set_can_target: false,
                                                        set_active: track!(model.changed(SlaveModel::status()), model.get_target_status(&SlaveStatusClass::MotionY) > JOYSTICK_DISPLAY_THRESHOLD),
                                                    },
                                                    attach(0, 2, 1, 1) = &ToggleButton {
                                                        set_icon_name: "go-previous-symbolic",
                                                        set_can_focus: false,
                                                        set_can_target: false,
                                                        set_active: track!(model.changed(SlaveModel::status()), model.get_target_status(&SlaveStatusClass::MotionX) < -JOYSTICK_DISPLAY_THRESHOLD),
                                                    },
                                                    attach(2, 2, 1, 1) = &ToggleButton {
                                                        set_icon_name: "go-next-symbolic",
                                                        set_can_focus: false,
                                                        set_can_target: false,
                                                        set_active: track!(model.changed(SlaveModel::status()), model.get_target_status(&SlaveStatusClass::MotionX) > JOYSTICK_DISPLAY_THRESHOLD),
                                                    },
                                                    attach(1, 3, 1, 1) = &ToggleButton {
                                                        set_icon_name: "go-down-symbolic",
                                                        set_can_focus: false,
                                                        set_can_target: false,
                                                        set_active: track!(model.changed(SlaveModel::status()), model.get_target_status(&SlaveStatusClass::MotionY) < -JOYSTICK_DISPLAY_THRESHOLD),
                                                    },
                                                },
                                            },
                                            append = &GtkBox {
                                                set_orientation: Orientation::Vertical,
                                                set_spacing: 5,
                                                set_hexpand: true,
                                                factory!(model.infos),
                                            },
                                            append = &CenterBox {
                                                set_hexpand: true,
                                                set_start_widget = Some(&Label) {
                                                    set_markup: "<b>深度锁定</b>",
                                                    set_margin_end: 5,
                                                },
                                                set_end_widget = Some(&Switch) {
                                                    set_active: track!(model.changed(SlaveModel::status()), model.get_target_status(&SlaveStatusClass::DepthLocked) != 0),
                                                    connect_state_set(sender) => move |_switch, state| {
                                                        send!(sender, SlaveMsg::SetSlaveStatus(SlaveStatusClass::DepthLocked, if state { 1 } else { 0 }));
                                                        Inhibit(false)
                                                    },
                                                },
                                            },
                                            append = &CenterBox {
                                                set_hexpand: true,
                                                set_start_widget = Some(&Label) {
                                                    set_markup: "<b>方向锁定</b>",
                                                    set_margin_end: 5,
                                                },
                                                set_end_widget = Some(&Switch) {
                                                    set_active: track!(model.changed(SlaveModel::status()), model.get_target_status(&SlaveStatusClass::DirectionLocked) != 0),
                                                    connect_state_set(sender) => move |_switch, state| {
                                                        send!(sender, SlaveMsg::SetSlaveStatus(SlaveStatusClass::DirectionLocked, if state { 1 } else { 0 }));
                                                        Inhibit(false)
                                                    },
                                                },
                                            },
                                        },
                                    },
                                },
                            },
                        },
                    },
                    connect_reveal_flap_notify(sender) => move |flap| {
                        send!(sender, SlaveMsg::SetConfigPresented(flap.reveals_flap()));
                    },
                },
            },
        }
    }
}

impl std::fmt::Debug for SlaveWidgets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.toast_overlay.fmt(f)
    }
}

pub enum SlaveMsg {
    ConfigUpdated,
    ToggleRecord,
    ToggleConnect,
    TogglePolling,
    PollingChanged(bool),
    RecordingChanged(bool),
    TakeScreenshot,
    AddInputSource(InputSource),
    RemoveInputSource(InputSource),
    SetSlaveStatus(SlaveStatusClass, i16),
    UpdateInputSources,
    ToggleDisplayInfo,
    InputReceived(InputSourceEvent),
    OpenFirmwareUpater,
    OpenParameterTuner,
    ErrorMessage(String),
    CommunicationError(String),
    ConnectionChanged(Option<async_std::sync::Arc<RpcClient>>),
    ShowToastMessage(String),
    CommunicationMessage(SlaveCommunicationMsg),
    InformationsReceived(HashMap<String, String>),
    SetConfigPresented(bool),
}

pub enum SlaveCommunicationMsg {
    ConnectionLost(RpcError),
    Disconnect,
    ControlUpdated(ControlPacket),
    Block(JoinHandle<Result<(), Box<dyn Error + Send>>>),
}

async fn communication_main_loop(
    input_rate: u16,
    rpc_client: Arc<RpcClient>,
    communication_sender: async_std::channel::Sender<SlaveCommunicationMsg>,
    communication_receiver: async_std::channel::Receiver<SlaveCommunicationMsg>,
    slave_sender: Sender<SlaveMsg>,
    status_info_udpate_interval: u64,
) -> Result<(), RpcError> {
    fn current_millis() -> u128 {
        // 获取当前时间距离UNIX纪元的持续时间，并转换为毫秒
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    }

    send!(
        slave_sender,
        SlaveMsg::ConnectionChanged(Some(rpc_client.clone()))
    ); // 发送连接状态改变消息

    let idle = async_std::sync::Arc::new(async_std::sync::Mutex::new(true)); // 空闲状态标
    let last_action_timestamp =
        async_std::sync::Arc::new(async_std::sync::Mutex::new(current_millis())); // 上次动作时间戳
    let control_packet =
        async_std::sync::Arc::new(async_std::sync::Mutex::new(None as Option<ControlPacket>)); // 控制数据包

    let receive_task = task::spawn(
        clone!(@strong communication_sender, @strong idle, @strong slave_sender, @strong rpc_client => async move {
            loop {
                if communication_sender.is_closed() {
                    return;
                }
                if *idle.lock().await {
                    // 请求获取信息
                    match rpc_client.request::<HashMap<String, String>>(METHOD_GET_INFO, None).await {
                        Ok(info) => send!(slave_sender, SlaveMsg::InformationsReceived(info)), // 发送接收到的信息消息
                        Err(error) => {
                            communication_sender.send(SlaveCommunicationMsg::ConnectionLost(error)).await.unwrap_or_default(); // 发送连接丢失消息
                            break;
                        },
                    }
                }
                task::sleep(Duration::from_millis(status_info_udpate_interval)).await; // 休眠一段时间后再继续循环
            }
        }),
    ); // 接收任务

    let control_send_task = task::spawn(
        clone!(@strong idle, @strong communication_sender, @strong rpc_client, @strong control_packet => async move {
            loop {
                if communication_sender.is_closed() {
                    return;
                }
                if *idle.lock().await {
                    let mut control_mutex = control_packet.lock().await;
                    if let Some(control) = control_mutex.as_ref() {
                        // 发送控制命令
                        for (method, params) in vec![
                            (METHOD_MOVE, Some(control.motion.to_rpc_params())),
                            (METHOD_SET_DEPTH_LOCKED, Some(control.depth_locked.to_rpc_params())),
                            (METHOD_SET_DIRECTION_LOCKED, Some(control.direction_locked.to_rpc_params())),
                            (METHOD_CATCH, Some(control.catch.to_rpc_params())),
                            (METHOD_LIGHT, Some(control.light.to_rpc_params())),
                        ].into_iter() {
                            match rpc_client.request::<()>(method, params).await {
                                Ok(_) => *control_mutex = None,
                                Err(err) => {
                                    communication_sender.send(SlaveCommunicationMsg::ConnectionLost(err)).await.unwrap_or_default(); // 发送连接丢失消息
                                }
                            }
                        }
                    }
                }
                task::sleep(Duration::from_millis(1000 / input_rate as u64)).await; // 休眠一段时间后再续循环
            }
        }),
    ); // 控制发送任务

    loop {
        match communication_receiver.recv().await {
            Ok(msg) if *idle.lock().await => match msg {
                SlaveCommunicationMsg::Disconnect => {
                    // 取消发送任务和接收任务
                    control_send_task.cancel().await;
                    receive_task.cancel().await;
                    // 发送连接状态改变消息
                    send!(slave_sender, SlaveMsg::ConnectionChanged(None));
                    // 关闭通接收器
                    communication_receiver.close();
                    // 退出循环
                    break;
                }
                SlaveCommunicationMsg::ConnectionLost(err) => {
                    // 取消发送任务和接收任务
                    control_send_task.cancel().await;
                    receive_task.cancel().await;
                    // 发送通信错误消息
                    send!(slave_sender, SlaveMsg::CommunicationError(err.to_string()));
                    // 关闭通信接收器
                    communication_receiver.close();
                    // 返回错误
                    return Err(err);
                }
                SlaveCommunicationMsg::ControlUpdated(control) => {
                    // 更新控制包
                    *control_packet.lock().await = Some(control);
                    // 更新最后操作时间戳
                    *last_action_timestamp.lock().await = current_millis();
                }
                SlaveCommunicationMsg::Block(blocker) => {
                    // 设置空闲状态为false
                    *idle.lock().await = false;
                    // 启动异任务
                    task::spawn(clone!(@strong idle => async move {
                        if let Err(err) = blocker.await {
                            eprintln!("模块异常退出：{}", err);
                        }
                        // 设置空闲状态true
                        *idle.lock().await = true;
                    }));
                }
            },
            _ => (),
        }
    }
    Ok(())
}

impl MicroModel for SlaveModel {
    type Msg = SlaveMsg;
    type Widgets = SlaveWidgets;
    type Data = (Sender<AppMsg>, WeakRef<ApplicationWindow>);
    fn update(
        &mut self,
        msg: SlaveMsg,
        (_parent_sender, app_window): &Self::Data,
        sender: Sender<SlaveMsg>,
    ) {
        self.reset();
        match msg {
            SlaveMsg::ConfigUpdated => {
                let config = self.get_mut_config().model().clone();
                send!(self.video.sender(), SlaveVideoMsg::ConfigUpdated(config));
            }
            SlaveMsg::ToggleConnect => {
                match self.get_connected() {
                    Some(true) => {
                        // 断开连接
                        self.set_connected(None);
                        self.config
                            .send(SlaveConfigMsg::SetConnected(None))
                            .unwrap();
                        let sender = self.get_communication_msg_sender().clone().unwrap();
                        task::spawn(async move {
                            sender
                                .send(SlaveCommunicationMsg::Disconnect)
                                .await
                                .expect("Communication main loop should be running");
                        });
                    }
                    Some(false) => {
                        // 连接
                        let url = self.config.model().get_slave_url().clone();
                        if let ("http", url_str) = (url.scheme(), url.as_str()) {
                            if let Ok(rpc_client) = RpcClientBuilder::default().build(url_str) {
                                let (comm_sender, comm_receiver) =
                                    async_std::channel::bounded::<SlaveCommunicationMsg>(128);
                                self.set_communication_msg_sender(Some(comm_sender.clone()));
                                let sender = sender.clone();
                                let control_sending_rate = 60;
                                self.set_connected(None);
                                self.config
                                    .send(SlaveConfigMsg::SetConnected(None))
                                    .unwrap();
                                let status_info_update_interval = 500;
                                async_std::task::spawn(async move {
                                    communication_main_loop(
                                        control_sending_rate,
                                        Arc::new(rpc_client),
                                        comm_sender,
                                        comm_receiver,
                                        sender.clone(),
                                        status_info_update_interval as u64,
                                    )
                                    .await
                                    .unwrap_or_default();
                                });
                            } else {
                                error_message(
                                    "错误",
                                    "无法创建 RPC 客户端。",
                                    app_window.upgrade().as_ref(),
                                );
                            }
                        } else {
                            error_message(
                                "错误",
                                "连接 URL 有误，请检查并修改后重试。",
                                app_window.upgrade().as_ref(),
                            );
                        }
                    }
                    None => (),
                }
            }
            SlaveMsg::TogglePolling => match self.get_polling() {
                Some(true) => {
                    // 如果值为 true
                    // 发送停止视频流水线消息给 self.video，并确保发送成功
                    self.video.send(SlaveVideoMsg::StopPipeline).unwrap();

                    // 设置轮为 None
                    self.set_polling(None);

                    // 发送设置轮询为 None 的消息给 self.config，并确保发送成功
                    self.config.send(SlaveConfigMsg::SetPolling(None)).unwrap();
                }

                Some(false) => {
                    // 如果值为 false
                    // 发送启动视频流水线消息给 self.video，并保发送成功
                    self.video.send(SlaveVideoMsg::StartPipeline).unwrap();

                    // 设置轮询为 None
                    self.set_polling(None);

                    // 发送设置轮询为 None 的消息给 self.config，并确保发送成功
                    self.config.send(SlaveConfigMsg::SetPolling(None)).unwrap();
                }

                None => (),
                // 如果值为 None，则不执行任何操作
            },
            SlaveMsg::AddInputSource(source) => {
                self.get_mut_input_sources().insert(source);
            }
            SlaveMsg::RemoveInputSource(source) => {
                self.get_mut_input_sources().remove(&source);
            }
            SlaveMsg::UpdateInputSources => {
                let _unuse = self.get_mut_input_system();
            }
            SlaveMsg::ToggleDisplayInfo => {
                self.set_slave_info_displayed(!*self.get_slave_info_displayed());
            }
            SlaveMsg::InputReceived(event) => {
                match event {
                    InputSourceEvent::ButtonChanged(button, pressed) => {
                        match SlaveStatusClass::from_button(button) {
                            Some(status_class @ SlaveStatusClass::RoboticArmOpen)
                            | Some(status_class @ SlaveStatusClass::LightOpen) => {
                                // 如果按钮被按下，设置目标状态为1，否则为0
                                self.set_target_status(&status_class, if pressed { 1 } else { 0 });
                            }
                            Some(status_class) => {
                                if pressed {
                                    // 如果按钮被按下，根据当前标状态的值设置相反的状态
                                    self.set_target_status(
                                        &status_class,
                                        !(self.get_target_status(&status_class) != 0) as i16,
                                    );
                                }
                            }
                            None => (),
                        }
                    }
                    InputSourceEvent::AxisChanged(axis, value) => {
                        match SlaveStatusClass::from_axis(axis) {
                            Some(status_class @ SlaveStatusClass::RoboticArmClose)
                            | Some(status_class @ SlaveStatusClass::LightClose) => match value {
                                1..=i16::MAX => {
                                    // 如果的值在1到最大值之，设置目标状态为1
                                    self.set_target_status(&status_class, 1);
                                }
                                i16::MIN..=0 => {
                                    // 如果轴的值在最小值到0之间，设置目标状态为0
                                    self.set_target_status(&status_class, 0);
                                }
                            },
                            Some(status_class) => {
                                // 根据轴值和方向设置标状态
                                self.set_target_status(
                                    &status_class,
                                    value.saturating_mul(
                                        if axis == Axis::LeftY || axis == Axis::RightY {
                                            -1
                                        } else {
                                            1
                                        },
                                    ),
                                );
                            }
                            None => (),
                        }
                    }
                }

                // 如果存在通信消息发送器
                if let Some(sender) = self.get_communication_msg_sender() {
                    // 根据当前状态映射创建制数据包
                    let control_packet =
                        ControlPacket::from_status_map(&self.get_status().lock().unwrap());
                    match sender.try_send(SlaveCommunicationMsg::ControlUpdated(control_packet)) {
                        Ok(_) => (),
                        Err(err) => println!("无法发送制输入：{}", err.to_string()),
                    }
                }
            }
            SlaveMsg::OpenFirmwareUpater => match self.get_rpc_client() {
                Some(rpc_client) => {
                    let component = MicroComponent::new(
                        SlaveFirmwareUpdaterModel::new(Deref::deref(rpc_client).clone()),
                        sender.clone(),
                    );
                    let window = component.root_widget();
                    window.set_transient_for(app_window.upgrade().as_ref());
                    window.set_visible(true);
                }
                None => {
                    error_message(
                        "错误",
                        "请确保下位机处于连接状态。",
                        app_window.upgrade().as_ref(),
                    );
                }
            },
            SlaveMsg::OpenParameterTuner => match self.get_rpc_client() {
                Some(rpc_client) => {
                    let component =
                        MicroComponent::new(SlaveParameterTunerModel::new(64, 250), sender.clone());
                    let window = component.root_widget();
                    window.set_transient_for(app_window.upgrade().as_ref());
                    window.set_visible(true);
                    send!(
                        component.sender(),
                        SlaveParameterTunerMsg::StartDebug(Deref::deref(rpc_client).clone())
                    );
                }
                None => {
                    error_message(
                        "错误",
                        "请确保下位机处于连接状态。",
                        app_window.upgrade().as_ref(),
                    );
                }
            },
            SlaveMsg::ErrorMessage(msg) => {
                error_message("错误", &msg, app_window.upgrade().as_ref());
            }
            SlaveMsg::CommunicationError(msg) => {
                send!(
                    sender,
                    SlaveMsg::ShowToastMessage(format!("下位机通讯错误：{}", msg))
                );
                send!(sender, SlaveMsg::ConnectionChanged(None));
            }
            SlaveMsg::ConnectionChanged(rpc_client) => {
                self.set_connected(Some(rpc_client.is_some()));
                self.config
                    .send(SlaveConfigMsg::SetConnected(Some(rpc_client.is_some())))
                    .unwrap();
                if rpc_client.is_none() {
                    self.set_communication_msg_sender(None);
                }
                self.set_rpc_client(rpc_client);
            }
            SlaveMsg::ShowToastMessage(msg) => {
                self.get_mut_toast_messages().borrow_mut().push_back(msg);
            }
            SlaveMsg::ToggleRecord => {
                let video = &self.video;
                if video.model().get_record_handle().is_none() {
                    let mut pathbuf = crate::preferences::get_video_path();
                    pathbuf.push(format!(
                        "{}.mkv",
                        DateTime::now_local()
                            .unwrap()
                            .format_iso8601()
                            .unwrap()
                            .replace(":", "-")
                    ));
                    send!(video.sender(), SlaveVideoMsg::StartRecord(pathbuf));
                } else {
                    send!(video.sender(), SlaveVideoMsg::StopRecord(None));
                }
                self.set_recording(None);
            }
            SlaveMsg::PollingChanged(polling) => {
                self.set_polling(Some(polling));
                send!(
                    self.config.sender(),
                    SlaveConfigMsg::SetPolling(Some(polling))
                );
                // send!(sender, SlaveMsg::InformationsReceived([("航向角".to_string(), "37°".to_string()), ("温度".to_string(), "25℃".to_string())].into_iter().collect())) // Debug
            }
            SlaveMsg::RecordingChanged(recording) => {
                if recording {
                    if *self.get_recording() == Some(false) {
                        self.set_sync_recording(true);
                    }
                } else {
                    self.set_sync_recording(false);
                }
                self.set_recording(Some(recording));
            }
            SlaveMsg::TakeScreenshot => {
                let mut pathbuf = crate::preferences::get_image_path();
                pathbuf.push(format!(
                    "{}.{}",
                    DateTime::now_local()
                        .unwrap()
                        .format_iso8601()
                        .unwrap()
                        .replace(":", "-"),
                    "jpg"
                ));
                send!(self.video.sender(), SlaveVideoMsg::SaveScreenshot(pathbuf));
            }
            SlaveMsg::CommunicationMessage(msg) => {
                if let Some(sender) = self.get_communication_msg_sender().as_ref() {
                    sender.try_send(msg).unwrap_or_default();
                }
            }
            SlaveMsg::InformationsReceived(info_map) => {
                let infos = self.get_mut_infos();
                let mut sorted_infos = info_map.into_iter().collect::<Vec<_>>();
                sorted_infos.sort();
                infos.clear();
                for (key, value) in sorted_infos.into_iter() {
                    infos.push(SlaveInfoModel {
                        key,
                        value,
                        ..Default::default()
                    });
                }
            }
            SlaveMsg::SetConfigPresented(presented) => self.set_config_presented(presented),
            SlaveMsg::SetSlaveStatus(which, value) => {
                self.set_target_status(&which, value);
                if let Some(sender) = self.get_communication_msg_sender() {
                    match sender.try_send(SlaveCommunicationMsg::ControlUpdated(
                        ControlPacket::from_status_map(&self.get_status().lock().unwrap()),
                    )) {
                        Ok(_) => (),
                        Err(err) => println!("无法更新机位状态：{}", err.to_string()),
                    }
                }
            }
        }
    }
}

pub struct MyComponent<T: MicroModel> {
    pub component: MicroComponent<T>,
}

impl<Model> MyComponent<Model>
where
    Model::Widgets: MicroWidgets<Model> + 'static,
    Model::Msg: 'static,
    Model::Data: 'static,
    Model: MicroModel + 'static,
{
    fn model(&self) -> std::cell::Ref<'_, Model> {
        self.component.model().unwrap()
    }
    #[allow(dead_code)]
    fn model_mut(&self) -> std::cell::RefMut<'_, Model> {
        self.component.model_mut().unwrap()
    }
    #[allow(dead_code)]
    fn widgets(&self) -> std::cell::RefMut<'_, Model::Widgets> {
        self.component.widgets().unwrap()
    }
}

impl<T: MicroModel> std::ops::Deref for MyComponent<T> {
    type Target = MicroComponent<T>;
    fn deref(&self) -> &MicroComponent<T> {
        &self.component
    }
}

impl<T: MicroModel> Debug for MyComponent<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MyComponent").finish()
    }
}

impl<Model> Default for MyComponent<Model>
where
    Model::Widgets: MicroWidgets<Model> + 'static,
    Model::Msg: 'static,
    Model::Data: Default + 'static,
    Model: MicroModel + Default + 'static,
{
    fn default() -> Self {
        MyComponent {
            component: MicroComponent::new(Model::default(), Model::Data::default()),
        }
    }
}

impl<Model> MyComponent<Model>
where
    Model::Widgets: MicroWidgets<Model> + 'static,
    Model::Msg: 'static,
    Model::Data: 'static,
    Model: MicroModel + 'static,
{
    pub fn new(model: Model, data: Model::Data) -> MyComponent<Model> {
        MyComponent {
            component: MicroComponent::new(model, data),
        }
    }
}

impl FactoryPrototype for MyComponent<SlaveModel> {
    type Factory = FactoryVec<Self>;
    type Widgets = ToastOverlay;
    type Root = ToastOverlay;
    type View = Grid;
    type Msg = AppMsg;

    fn init_view(&self, _index: &usize, _sender: Sender<AppMsg>) -> ToastOverlay {
        self.component.root_widget().clone()
    }

    fn position(&self, index: &usize) -> GridPosition {
        let index = *index as i32;
        let row = index / 3;
        let column = index % 3;
        GridPosition {
            column,
            row,
            width: 1,
            height: 1,
        }
    }

    fn view(&self, _index: &usize, _widgets: &ToastOverlay) {
        self.component.update_view().unwrap();
    }

    fn root_widget(widgets: &ToastOverlay) -> &ToastOverlay {
        widgets
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct MotionPacket {
    x: f32,
    y: f32,
    z: f32,
    rot: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ControlPacket {
    motion: MotionPacket,
    catch: f32,
    light: f32,
    depth_locked: bool,
    direction_locked: bool,
}

impl ControlPacket {
    pub fn from_status_map(status_map: &HashMap<SlaveStatusClass, i16>) -> ControlPacket {
        fn map_value(value: &i16) -> f32 {
            match *value {
                0 => 0.0,
                1..=i16::MAX => *value as f32 / i16::MAX as f32,
                i16::MIN..=-1 => *value as f32 / i16::MIN as f32 * -1.0,
            }
        }
        ControlPacket {
            motion: MotionPacket {
                x: map_value(status_map.get(&SlaveStatusClass::MotionX).unwrap_or(&0)),
                y: map_value(status_map.get(&SlaveStatusClass::MotionY).unwrap_or(&0)),
                z: map_value(status_map.get(&SlaveStatusClass::MotionZ).unwrap_or(&0)),
                rot: map_value(
                    status_map
                        .get(&SlaveStatusClass::MotionRotate)
                        .unwrap_or(&0),
                ),
            },
            catch: (*status_map
                .get(&SlaveStatusClass::RoboticArmOpen)
                .unwrap_or(&0)
                * 1
                + *status_map
                    .get(&SlaveStatusClass::RoboticArmClose)
                    .unwrap_or(&0)
                    * -1) as f32,
            light: (*status_map.get(&SlaveStatusClass::LightOpen).unwrap_or(&0) * 1
                + *status_map.get(&SlaveStatusClass::LightClose).unwrap_or(&0) * -1)
                as f32,
            depth_locked: status_map
                .get(&SlaveStatusClass::DepthLocked)
                .map(|x| *x >= 1)
                .unwrap_or(false),
            direction_locked: status_map
                .get(&SlaveStatusClass::DirectionLocked)
                .map(|x| *x >= 1)
                .unwrap_or(false),
        }
    }
}

impl ToString for ControlPacket {
    fn to_string(&self) -> String {
        serde_json::to_string_pretty(self).unwrap()
    }
}

pub trait AsRpcParams {
    fn to_rpc_params(&self) -> RpcParams;
}

impl<T: Serialize> AsRpcParams for T {
    fn to_rpc_params(&self) -> RpcParams {
        match serde_json::to_value(self).unwrap() {
            serde_json::Value::Object(map) => map
                .into_iter()
                .map(|(key, value)| ((Box::leak(Box::new(key)) as &'static str), value))
                .collect::<BTreeMap<_, _>>()
                .into(),
            serde_json::Value::Array(vec) => vec.into(),
            x => vec![x].into(),
        }
    }
}
