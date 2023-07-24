/* input.rs
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

use std::{
    cell::RefCell,
    collections::HashMap,
    fmt::Debug,
    ops::Deref,
    rc::Rc,
    sync::{Arc, Mutex},
    time::Duration,
};

use glib::{Continue, Sender};

use fragile::Fragile;
use sdl2::{event::Event, GameControllerSubsystem, Sdl};

use lazy_static::lazy_static;

pub type Button = sdl2::controller::Button;
pub type Axis = sdl2::controller::Axis;
pub type GameController = sdl2::controller::GameController;

#[derive(Hash, Debug, PartialEq, Clone, Eq)]
pub enum InputSource {
    GameController(u32),
}

pub enum InputSystemMessage {
    RetrieveJoystickList,
    Connect(u32),
}

#[derive(Debug, Clone)]
pub enum InputSourceEvent {
    ButtonChanged(Button, bool),
    AxisChanged(Axis, i16),
}

pub struct InputEvent(pub InputSource, pub InputSourceEvent);

lazy_static! {
    pub static ref SDL: Result<Fragile<Sdl>, String> = sdl2::init().map(Fragile::new);
}

pub struct InputSystem {
    pub sdl: Sdl,
    pub game_controller_subsystem: GameControllerSubsystem,
    pub game_controllers: Arc<Mutex<HashMap<u32, GameController>>>, // GameController 在 drop 时会自动断开连接，因此容器来保存
    pub event_sender: Rc<RefCell<Option<Sender<InputEvent>>>>,
    running: Arc<Mutex<bool>>,
}

impl InputSystem {
    pub fn get_sources(&self) -> Result<Vec<(InputSource, String)>, String> {
        // 获取游戏控制器子系统中的游戏手柄数量
        let num = self.game_controller_subsystem.num_joysticks()?;

        // 构建包含游戏手柄输入源和名称的元向量，并返回结果
        Ok((0..num)
            .map(|index| {
                (
                    InputSource::GameController(index), // 游戏手柄输入源
                    self.game_controller_subsystem
                        .name_for_index(index)
                        .unwrap_or("未知备".to_string()), // 游戏手名称，如果获取失败则使用默认值"未知设备"
                )
            })
            .collect())
    }
}

impl Debug for InputSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputSystem")
            .field("game_controller_subsystem", &self.game_controller_subsystem)
            .field("event_sender", &self.event_sender)
            .field("running", &self.running)
            .finish()
    }
}

impl Default for InputSystem {
    fn default() -> Self {
        let sdl_fragile = Deref::deref(&SDL).clone().unwrap();
        let sdl = sdl_fragile.get();
        let game_controller_subsystem = sdl.game_controller().unwrap();
        InputSystem::new(&sdl, &game_controller_subsystem)
    }
}

impl InputSystem {
    pub fn new(sdl: &Sdl, game_controller_subsystem: &GameControllerSubsystem) -> Self {
        // 创建一个可选的事件发送器，用发送输入事件
        let event_sender: Rc<RefCell<Option<Sender<InputEvent>>>> = Rc::new(RefCell::new(None));

        Self {
            // 克隆传入的 Sdl 实例并存储在结构体中
            sdl: sdl.clone(),
            // 克隆传入的 GameControllerSubsystem 实例并存储在结构体中
            game_controller_subsystem: game_controller_subsystem.clone(),
            // 创建一个互斥锁保护的哈希映射，用于存储游戏控制器
            game_controllers: Arc::new(Mutex::new(HashMap::new())),
            // 存储事件发送器的引用计数智能指针
            event_sender,
            // 创建一个互斥锁保护的布尔值，表示游戏是否正在运行
            running: Arc::new(Mutex::new(false)),
        }
    }

    pub fn run(&self) {
        // 检查是否正在运行，如果是则返回
        if *self.running.lock().unwrap() {
            return;
        }

        // 获取可用的游戏控制器数量
        let available = self
            .game_controller_subsystem
            .num_joysticks()
            .map_err(|e| format!("无法枚举游戏控制器：{}", e))
            .unwrap();

        // 遍历可用的游戏制器，并将其添加到游戏控制器集合
        for (id, game_controller) in (0..available).filter_map(|id| {
            self.game_controller_subsystem
                .open(id)
                .ok()
                .map(|c| (id, c))
        }) {
            self.game_controllers
                .lock()
                .unwrap()
                .insert(id, game_controller);
        }

        // 克隆要的变量
        let sdl = self.sdl.clone();
        let sender = self.event_sender.clone();
        let running = self.running.clone();

        // 设置运行状态为 true
        *self.running.lock().unwrap() = true;

        // 克必要的变量
        let game_controller_subsystem = self.game_controller_subsystem.clone();
        let game_controllers = self.game_controllers.clone();
        glib::timeout_add_local(Duration::from_millis(16), move || {
            let mut event_pump = sdl.event_pump().expect("Cannot get event pump from SDL");
            if let Some(sender) = sender.as_ref().borrow().as_ref() {
                for event in event_pump.poll_iter() {
                    match event {
                        Event::ControllerAxisMotion {
                            axis, which, value, ..
                        } => sender
                            .send(InputEvent(
                                InputSource::GameController(which),
                                InputSourceEvent::AxisChanged(axis, value),
                            ))
         .unwrap(), // 发送输入事件到发送器
                        Event::ControllerButtonDown { button, which, .. } => sender
                            .send(InputEvent(
                                InputSource::GameController(which),
                                InputSourceEvent::ButtonChanged(button, true),
                            ))
                            .unwrap(), // 发送输入事件到发送器
                        Event::ControllerButtonUp { button, which, .. } => sender
                            .send(InputEvent(
                                InputSource::GameController(which),
                                InputSourceEvent::ButtonChanged(button, false),
                            ))
                            .unwrap(), // 发送输入事件到发送
                        Event::ControllerDeviceAdded { which, .. } => {
                            if let Ok(game_controller) = game_controller_subsystem.open(which) {
                                game_controllers
                                    .lock()
                                    .unwrap()
                                    .insert(which, game_controller);
                            }
                        } // 添加游戏控制器设备
                        Event::ControllerDeviceRemoved { which, .. } => {
                            game_controllers.lock().unwrap().remove(&which);
                        } // 移除游戏控制器设备
                        Event::Quit { .. } => break, // 退出事件，跳出循环
                        _ => (), // 其他事件，忽略
                    }
                }
            } else {
                event_pump.poll_iter().last(); // 如果没有发送器，只处理最后一个事件
            }
            Continue(*running.clone().lock().unwrap()) // 继续执行定时器回调函数
        });
        
    }

    pub fn stop(&self) {
        *self.running.lock().unwrap() = false;
    }
}
