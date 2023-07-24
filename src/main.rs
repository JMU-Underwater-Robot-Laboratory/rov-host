/* main.rs
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
use gtk::{
    AboutDialog, Align, Box as GtkBox, Grid, Inhibit, License, MenuButton, Orientation,
    ToggleButton,
};
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
use crate::slave::{
    slave_config::SlaveConfigModel, MyComponent, SlaveModel, SlaveMsg,
};

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
            set_website: Some("https://github.com/BohongHuang/rov-host"),
            set_authors: &["黄博宏 https://bohonghuang.github.io", "彭剑锋 https://qff233.com"],
            set_program_name: Some("水下机器人上位机"),
            set_copyright: Some("© 2021-2023 集美大学水下智能创新实验室"),
            set_comments: Some("跨平台的水下机器人上位机程序"),
            set_logo_icon_name: Some("input-gaming"),
            set_version: Some(env!("CARGO_PKG_VERSION")),
            set_license_type: License::Gpl30,
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
                    pack_end = &ToggleButton {
                        set_icon_name: "view-fullscreen-symbolic",
                        set_tooltip_text: Some("切换全屏模式"),
                        set_active: track!(model.changed(AppModel::fullscreened()), *model.get_fullscreened()),
                        connect_clicked(sender) => move |button| {
                            send!(sender, AppMsg::SetFullscreened(button.is_active()));
                        }
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

        let app_group = RelmActionGroup::<AppActionGroup>::new();

        let action_preferences: RelmAction<PreferencesAction> =
            RelmAction::new_stateless(clone!(@strong sender => move |_| {
                send!(sender, AppMsg::OpenPreferencesWindow);
            }));
        let action_about: RelmAction<AboutDialogAction> =
            RelmAction::new_stateless(clone!(@strong sender => move |_| {
                send!(sender, AppMsg::OpenAboutDialog);
            }));

        app_group.add_action(action_preferences);
        app_group.add_action(action_about);
        app_window.insert_action_group("main", Some(&app_group.into_action_group()));
        send!(sender, AppMsg::NewSlave(app_window.clone().downgrade()));
        let (input_event_sender, input_event_receiver) = MainContext::channel(PRIORITY_DEFAULT);
        *model.input_system.event_sender.borrow_mut() = Some(input_event_sender);

        input_event_receiver.attach(
            None,
            clone!(@strong sender => move |event| {
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
                components.about.root_widget().present();
            }
            AppMsg::OpenPreferencesWindow => {
                components.preferences.root_widget().present();
            }
            AppMsg::NewSlave(app_window) => {
                let index = self.get_slaves().len() as u8;
                let mut slave_url: url::Url = self
                    .get_preferences()
                    .borrow()
                    .get_default_slave_url()
                    .clone();
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
                let (input_event_sender, input_event_receiver) =
                    MainContext::channel(PRIORITY_DEFAULT);
                let (slave_event_sender, slave_event_receiver) =
                    MainContext::channel(PRIORITY_DEFAULT);
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
                let slave = SlaveModel::new(
                    slave_config,
                    self.get_preferences().clone(),
                    &slave_event_sender,
                    input_event_sender,
                );
                let component = MyComponent::new(slave, (sender.clone(), app_window));
                let component_sender = component.sender().clone();
                input_event_receiver.attach(
                    None,
                    clone!(@strong component_sender => move |event| {
                        component_sender.send(SlaveMsg::InputReceived(event)).unwrap();
                        Continue(true)
                    }),
                );
                slave_event_receiver.attach(
                    None,
                    clone!(@strong component_sender => move |event| {
                        component_sender.send(event).unwrap();
                        Continue(true)
                    }),
                );
                self.get_mut_slaves().push(component);
                self.set_sync_recording(Some(false));
            }
            AppMsg::PreferencesUpdated(preferences) => {
                *self.get_mut_preferences().borrow_mut() = preferences;
            }
            AppMsg::DispatchInputEvent(InputEvent(source, event)) => {
                for slave in self.slaves.iter() {
                    let slave_model = slave.model().unwrap();
                    if slave_model.get_input_sources().contains(&source) {
                        slave_model.input_event_sender.send(event.clone()).unwrap();
                    }
                }
            }
            AppMsg::StopInputSystem => {
                self.input_system.stop();
            }
            AppMsg::SetFullscreened(fullscreened) => self.set_fullscreened(fullscreened),
        }
        true
    }
}

fn main() {
    gst::init().expect("无法初始化 GStreamer");
    gtk::init().map(|_| adw::init()).expect("无法初始化 GTK4");

    let setting = gtk::Settings::default().unwrap();
    setting.set_gtk_theme_name(Some("Windows10"));

    let model = AppModel {
        preferences: Rc::new(RefCell::new(PreferencesModel::load_or_default())),
        ..Default::default()
    };
    model.input_system.run();
    let relm = RelmApp::new(model);
    relm.run()
}
