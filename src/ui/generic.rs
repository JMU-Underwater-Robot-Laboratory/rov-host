/* generic.rs
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

use std::path::PathBuf;

use gtk::{
    prelude::*, FileChooserAction, FileChooserNative, FileFilter, MessageDialog, ResponseType,
};

pub fn select_path<T, F>(
    action: FileChooserAction,
    filters: &[FileFilter],
    parent_window: &T,
    callback: F,
) -> FileChooserNative
where
    T: IsA<gtk::Window>,
    F: 'static + Fn(Option<PathBuf>) -> (),
{
    relm4_macros::view! {
           // 创建一个本地文件选择器
           file_chooser = FileChooserNative {
               // 设置文件选择器的动作类型
               set_action: action,
               // 添加过滤器到文件选择器
               add_filter: iterate!(filters),
               // 设置是否允许创建文件夹
               set_create_folders: true,
               // 设置取消按钮的标签为"取消"
               set_cancel_label: Some("取消"),
               // 设置受按钮的标签为"打开"
               set_accept_label: Some("打开"),
               // 设置文件选择器为模态对话框
               set_modal: true,
               // 设置文件选择器的父窗
               set_transient_for: Some(parent_window),
               // 连接响应信号，当用户点击按钮时触发回调函数
               connect_response => move |dialog, res_ty| {
                   match res_ty {
                       gtk::ResponseType::Accept => {
                           if let Some(file) = dialog.file() {
                               if let Some(path) = file.path() {
                                   // 调用回调函数并递选中的路径
                                   callback(Some(path));
                                   return;
                               }
                           }
                       },
                       gtk::ResponseType::Cancel => {
                           // 调用回调函数并传递空路径
                           callback(None);
                       },
                       _ => (),
                   }
               },
           }
    }
    // 显示文件选择器
    file_chooser.show();
    // 返回文件选择器实例
    file_chooser
}

pub fn error_message<T>(title: &str, msg: &str, window: Option<&T>) -> MessageDialog
where
    T: IsA<gtk::Window>,
{
    relm4_macros::view! {
        // 创建一个消息对话框
        dialog = MessageDialog {
            // 设置消息类型为错误
            set_message_type: gtk::MessageType::Error,
            // 设置消息文本
            set_text: Some(msg),
            // 设置对话框标题
            set_title: Some(title),
            // 设置对话框为模对话框
            set_modal: true,
            // 设置对话框的父窗口
            set_transient_for: window,
            // 添加一个按钮到对话框，按钮文本为"确定"，响应类型为Ok
            add_button: args!("确定", ResponseType::Ok),
            // 连接响应信号，当用户点击按钮时销毁对话框
            connect_response => |dialog, _response| {
                dialog.destroy();
            }
        }
    }
    // 显示对话框
    dialog.show();
    // 返回对话框实例
    dialog
}
