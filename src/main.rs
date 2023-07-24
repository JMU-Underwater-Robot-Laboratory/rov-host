/* main.rs
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

pub mod async_glib;
pub mod function;
pub mod input;
pub mod preferences;
pub mod prelude;
pub mod slave;
pub mod ui;

use std::{cell::RefCell, net::Ipv4Addr, rc::Rc, str::FromStr};

use adw::{prelude::*, ApplicationWindow, CenteringPolicy, HeaderBar};
use glib::{clone, MainContext, Sender, WeakRef, PRIORITY_DEFAULT};
use gtk::{AboutDialog, Align, Box as GtkBox, Grid, Inhibit, License, MenuButton, Orientation};
use relm4::{
    actions::{RelmAction, RelmActionGroup},
    factory::FactoryVec,
    new_action_group, new_stateless_action, send, AppUpdate, ComponentUpdate, Model, RelmApp,
    RelmComponent, Widgets,
};
use relm4_macros::widget;

use derivative::*;
use serde::{Deserialize, Serialize};
use strum_macros::EnumIter;

use crate::input::{InputEvent, InputSystem};
use crate::preferences::PreferencesModel;
use crate::slave::{slave_config::SlaveConfigModel, MyComponent, SlaveModel, SlaveMsg};

struct AboutModel {}
enum AboutMsg {}
impl Model for AboutModel {
    type Msg = AboutMsg;
    type Widgets = AboutWidgets;
    type Components = ();
}

#[widget]
impl Widgets<AboutModel, AppModel> for AboutWidgets {
    view! {
        dialog = AboutDialog {
            set_transient_for: parent!(Some(&parent_widgets.app_window)),
            set_destroy_with_parent: true,
            set_modal: true,
            connect_close_request => move |window| {
                window.hide();
                Inhibit(true)
            },
            set_website: Some("https://JMU-Underwater.github.io"),
            set_authors: &["黄博宏 https://bohonghuang.github.io", "彭剑锋 https://qff233.com"],
            set_program_name: Some("水下机器人上位机"),
            set_copyright: Some("© 2021-2023 集美大学水下智能创新实验室"),
            set_comments: Some("跨平台的水下机器人上位机程序"),
            set_logo_icon_name: Some("input-gaming"),
            set_version: Some(env!("CARGO_PKG_VERSION")),
            set_license_type: License::Agpl30,
        }
    }
}

impl ComponentUpdate<AppModel> for AboutModel {
    fn init_model(_parent_model: &AppModel) -> Self {
        AboutModel {}
    }
    fn update(
        &mut self,
        _msg: AboutMsg,
        _components: &(),
        _sender: Sender<AboutMsg>,
        _parent_sender: Sender<AppMsg>,
    ) {
    }
}

#[derive(EnumIter, PartialEq, Clone, Copy, Debug, Serialize, Deserialize)]
pub enum AppColorScheme {
    Light,
}

impl ToString for AppColorScheme {
    fn to_string(&self) -> String {
        match self {
            AppColorScheme::Light => "浅色",
        }
        .to_string()
    }
}

impl Default for AppColorScheme {
    fn default() -> Self {
        Self::Light
    }
}

#[tracker::track]
#[derive(Derivative)]
#[derivative(Default)]
pub struct AppModel {
    #[derivative(Default(value = "Some(false)"))]
    sync_recording: Option<bool>,
    fullscreened: bool,
    #[no_eq]
    #[derivative(Default(value = "FactoryVec::new()"))]
    slaves: FactoryVec<MyComponent<SlaveModel>>,
    #[no_eq]
    preferences: Rc<RefCell<PreferencesModel>>,
    #[no_eq]
    input_system: Rc<InputSystem>,
}

impl Model for AppModel {
    type Msg = AppMsg;
    type Widgets = AppWidgets;
    type Components = AppComponents;
}

new_action_group!(AppActionGroup, "main");
new_stateless_action!(PreferencesAction, AppActionGroup, "preferences");
new_stateless_action!(AboutDialogAction, AppActionGroup, "about");

#[widget(pub)]
impl Widgets<AppModel, ()> for AppWidgets {
    view! {
        app_window = ApplicationWindow::default() {
            set_title: Some("水下机器人上位机"),
            set_default_width: 1280,
            set_default_height: 720,
            set_icon_name: Some("input-gaming"),
            set_fullscreened: track!(model.changed(AppModel::fullscreened()), *model.get_fullscreened()),
            set_content = Some(&GtkBox) {
                set_orientation: Orientation::Vertical,
                append = &HeaderBar {
                    set_centering_policy: CenteringPolicy::Strict,
                    pack_end = &MenuButton {
                        set_menu_model: Some(&main_menu),
                        set_icon_name: "open-menu-symbolic",
                        set_focus_on_click: false,
                        set_valign: Align::Center,
                    },
                },
                append: body_stack = &Grid {
                    set_column_homogeneous: true,
                    set_row_homogeneous: true,
                    factory!(model.slaves),
                },
            },
            connect_close_request(sender) => move |_window| {
                send!(sender, AppMsg::StopInputSystem);
                Inhibit(false)
            },
        }
    }

    menu! {
        main_menu: {
            "首选项"     => PreferencesAction,
            "关于"       => AboutDialogAction,
        }
    }

    fn post_init() {
        // 创建一个新 RelmActionGroup 对象，用于管理作
        let app_group = RelmActionGroup::<AppActionGroup>::new();

        // 定义 action_preferences 动作，当触时发送 AppMsg::OpenPreferencesWindow 消息给 sender
        let action_preferences: RelmAction<PreferencesAction> =
            RelmAction::new_stateless(clone!(@strong sender => move |_| {
                send!(sender, AppMsg::OpenPreferencesWindow);
            }));

        // 定义 action_about 动作，当触发时发送 AppMsg::OpenAboutDialog 消息给 sender
        let action_about: RelmAction<AboutDialogAction> =
            RelmAction::new_stateless(clone!(@strong sender => move |_| {
                send!(sender, AppMsg::OpenAboutDialog);
            }));

        // 将动作添加到 app_group 中
        app_group.add_action(action_preferences);
        app_group.add_action(action_about);

        // 将 app_group 插入到 app_window 中的 "main" 动作组中
        app_window.insert_action_group("main", Some(&app_group.into_action_group()));

        // 发送 AppMsg::Slave 消息给 sender，payload 为 app_window 的弱引用
        send!(sender, AppMsg::NewSlave(app_window.clone().downgrade()));

        // 创建一个通道，返回发送器和接收器
        let (input_event_sender, input_event_receiver) = MainContext::channel(PRIORITY_DEFAULT);

        // 将 input_event_sender 分配给 model.input_system.event_sender
        *model.input_system.event_sender.borrow_mut() = Some(input_event_sender);

        // 附加 input_event_receiver 来处理输入事件
        input_event_receiver.attach(
            None,
            clone!(@strong sender => move |event| {
                // 发送 AppMsg::DispatchInputEvent 消息给 senderpayload 为接收到事件
                send!(sender, AppMsg::DispatchInputEvent(event));
                Continue(true)
            }),
        );
    }
}

pub enum AppMsg {
    NewSlave(WeakRef<ApplicationWindow>),
    DispatchInputEvent(InputEvent),
    PreferencesUpdated(PreferencesModel),
    SetFullscreened(bool),
    OpenAboutDialog,
    OpenPreferencesWindow,
    StopInputSystem,
}

#[derive(relm4_macros::Components)]
pub struct AppComponents {
    about: RelmComponent<AboutModel, AppModel>,
    preferences: RelmComponent<PreferencesModel, AppModel>,
}

impl AppUpdate for AppModel {
    fn update(&mut self, msg: AppMsg, components: &AppComponents, sender: Sender<AppMsg>) -> bool {
        self.reset();
        match msg {
            AppMsg::OpenAboutDialog => {
                // 打开关于对话框
                components.about.root_widget().present();
            }
            AppMsg::OpenPreferencesWindow => {
                // 打开首选项窗口
                components.preferences.root_widget().present();
            }
            AppMsg::NewSlave(app_window) => {
                // 创建新的从属应用窗口
                let index = self.get_slaves().len() as u8;
                let mut slave_url: url::Url = self
                    .get_preferences()
                    .borrow()
                    .get_default_slave_url()
                    .clone();

                // 如果从属 URL 的主机部是有效的 IPv4 地址，则根据索引增加 IP 地址的最一位
                if let Some(ip) = slave_url
                    .host_str()
                    .and_then(|str| Ipv4Addr::from_str(str).ok())
                {
                    let mut ip_octets = ip.octets();
                    ip_octets[3] = ip_octets[3].wrapping_add(index);
                    slave_url
                        .set_host(Some(Ipv4Addr::from(ip_octets).to_string().as_str()))
                        .unwrap_or_default();
                }

                // 根据索引增加视频 URL 的端口号
                let mut video_url = self
                    .get_preferences()
                    .borrow()
                    .get_default_video_url()
                    .clone();
                if let Some(port) = video_url.port() {
                    video_url
                        .set_port(Some(port.wrapping_add(index as u16)))
                        .unwrap();
                }

                // 创建输入事件通道和从属事件通道
                let (input_event_sender, input_event_receiver) =
                    MainContext::channel(PRIORITY_DEFAULT);
                let (slave_event_sender, slave_event_receiver) =
                    MainContext::channel(PRIORITY_DEFAULT);

                // 根据首选项创建从属配置模型
                let mut slave_config =
                    SlaveConfigModel::from_preferences(&self.preferences.borrow());
                slave_config.set_slave_url(slave_url);
                slave_config.set_video_url(video_url);
                slave_config.set_keep_video_display_ratio(
                    *self
                        .get_preferences()
                        .borrow()
                        .get_default_keep_video_display_ratio(),
                );

                // 创建属模型并关联事件发送器
                let slave = SlaveModel::new(
                    slave_config,
                    self.get_preferences().clone(),
                    &slave_event_sender,
                    input_event_sender,
                );

                // 创建组件并获取其事件发送器
                let component = MyComponent::new(slave, (sender.clone(), app_window));
                let component_sender = component.sender().clone();

                // 将输入事件接收器与组件的事件发送器关联
                input_event_receiver.attach(
                    None,
                    clone!(@strong component_sender => move |event| {
                        component_sender.send(SlaveMsg::InputReceived(event)).unwrap();
                        Continue(true)
                    }),
                );

                // 将从属事件接收器与组件的事件发送器关联
                slave_event_receiver.attach(
                    None,
                    clone!(@strong component_sender => move |event| {
                        component_sender.send(event).unwrap();
                        Continue(true)
                    }),
                );

                // 将组件添加到从属列表中
                self.get_mut_slaves().push(component);

                // 设置同步录制状态为 false
                self.set_sync_recording(Some(false));
            }
            AppMsg::PreferencesUpdated(preferences) => {
                // 更新首选项
                *self.get_mut_preferences().borrow_mut() = preferences;
            }
            AppMsg::DispatchInputEvent(InputEvent(source, event)) => {
                // 遍历所有从属模块
                for slave in self.slaves.iter() {
                    let slave_model = slave.model().unwrap();
                    // 获取从属模块的输入源列表，并检查是否包含当前事件的来源
                    if slave_model.get_input_sources().contains(&source) {
                        // 将事件发送给从属模块的输入事件发送器
                        slave_model.input_event_sender.send(event.clone()).unwrap();
                    }
                }
            }
            AppMsg::StopInputSystem => {
                // 停止输入系统
                self.input_system.stop();
            }
            AppMsg::SetFullscreened(fullscreened) => self.set_fullscreened(fullscreened),
        }
        true
    }
}

fn main() {
    // 初始化GStreamer库
    gst::init().expect("无初始化 GStreamer");

    // 初始化GTK4库和adw库
    gtk::init().map(|_| adw::init()).expect("无法初始化 GTK4");

    // 获取默认GTK设置
    let setting = gtk::Settings::default().unwrap();

    // 设置GTK主题为"Windows10"
    setting.set_gtk_theme_name(Some("Windows10"));

    // 创建应用程序模型例
    let model = AppModel {
        // 初始化偏好设置，从存储中加载或创建默认设置
        preferences: Rc::new(RefCell::new(PreferencesModel::load_or_default())),
        ..Default::default()
    };

    // 运行输入系统
    model.input_system.run();

    // 创建并运行Relm应用程序
    let relm = RelmApp::new(model);
    relm.run();
}
