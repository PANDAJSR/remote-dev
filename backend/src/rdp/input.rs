use enigo::{
    Axis, Button, Coordinate,
    Direction::{Click, Press, Release},
    Enigo, Key, Keyboard, Mouse, Settings,
};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// 输入事件类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InputEvent {
    #[serde(rename = "mouse_move")]
    MouseMove { x: i32, y: i32, absolute: bool },

    #[serde(rename = "mouse_click")]
    MouseClick {
        button: MouseButton,
        action: ClickAction,
    },

    #[serde(rename = "mouse_scroll")]
    MouseScroll { delta_x: i32, delta_y: i32 },

    #[serde(rename = "key_press")]
    KeyPress {
        key: String,
        modifiers: Vec<ModifierKey>,
    },

    #[serde(rename = "key_release")]
    KeyRelease { key: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClickAction {
    Press,
    Release,
    Click,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModifierKey {
    Shift,
    Ctrl,
    Alt,
    Meta,
}

/// 输入控制器
pub struct InputController {
    enigo: Arc<Mutex<Enigo>>,
    screen_width: u32,
    screen_height: u32,
}

impl InputController {
    pub fn new(screen_width: u32, screen_height: u32) -> anyhow::Result<Self> {
        let settings = Settings::default();
        let enigo = Enigo::new(&settings)?;

        Ok(Self {
            enigo: Arc::new(Mutex::new(enigo)),
            screen_width,
            screen_height,
        })
    }

    /// 处理输入事件
    pub fn handle_event(&self, event: InputEvent) -> anyhow::Result<()> {
        let mut enigo = self.enigo.lock().unwrap();

        match event {
            InputEvent::MouseMove { x, y, absolute } => {
                if absolute {
                    // 绝对坐标 - 映射到屏幕分辨率
                    let scaled_x = (x as f32 * (self.screen_width as f32 / 1920.0)) as i32;
                    let scaled_y = (y as f32 * (self.screen_height as f32 / 1080.0)) as i32;
                    enigo.move_mouse(scaled_x, scaled_y, Coordinate::Abs)?;
                } else {
                    enigo.move_mouse(x, y, Coordinate::Rel)?;
                }
            }

            InputEvent::MouseClick { button, action } => {
                let btn = match button {
                    MouseButton::Left => Button::Left,
                    MouseButton::Right => Button::Right,
                    MouseButton::Middle => Button::Middle,
                };

                let direction = match action {
                    ClickAction::Press => Press,
                    ClickAction::Release => Release,
                    ClickAction::Click => Click,
                };

                enigo.button(btn, direction)?;
            }

            InputEvent::MouseScroll { delta_x, delta_y } => {
                // 垂直滚动
                if delta_y != 0 {
                    let scroll_amount = delta_y.clamp(-10, 10);
                    enigo.scroll(scroll_amount, Axis::Vertical)?;
                }
                // 水平滚动
                if delta_x != 0 {
                    let scroll_amount = delta_x.clamp(-10, 10);
                    enigo.scroll(scroll_amount, Axis::Horizontal)?;
                }
            }

            InputEvent::KeyPress { key, modifiers } => {
                // 先按下修饰键
                for modifier in &modifiers {
                    match modifier {
                        ModifierKey::Shift => enigo.key(Key::Shift, Press)?,
                        ModifierKey::Ctrl => enigo.key(Key::Control, Press)?,
                        ModifierKey::Alt => enigo.key(Key::Alt, Press)?,
                        ModifierKey::Meta => enigo.key(Key::Meta, Press)?,
                    }
                }

                // 按下主键
                if let Some(enigo_key) = Self::parse_key(&key) {
                    enigo.key(enigo_key, Press)?;
                }
            }

            InputEvent::KeyRelease { key } => {
                if let Some(enigo_key) = Self::parse_key(&key) {
                    enigo.key(enigo_key, Release)?;
                }

                // 释放所有修饰键
                enigo.key(Key::Shift, Release)?;
                enigo.key(Key::Control, Release)?;
                enigo.key(Key::Alt, Release)?;
                enigo.key(Key::Meta, Release)?;
            }
        }

        Ok(())
    }

    /// 解析键名字符串为 enigo::Key
    fn parse_key(key_str: &str) -> Option<Key> {
        let key_lower = key_str.to_lowercase();

        // 特殊键映射
        match key_lower.as_str() {
            // 功能键
            "enter" | "return" => return Some(Key::Return),
            "escape" | "esc" => return Some(Key::Escape),
            "backspace" => return Some(Key::Backspace),
            "tab" => return Some(Key::Tab),
            "space" | " " => return Some(Key::Space),
            "delete" | "del" => return Some(Key::Delete),
            "home" => return Some(Key::Home),
            "end" => return Some(Key::End),
            "pageup" => return Some(Key::PageUp),
            "pagedown" => return Some(Key::PageDown),

            // 方向键
            "up" | "arrowup" => return Some(Key::UpArrow),
            "down" | "arrowdown" => return Some(Key::DownArrow),
            "left" | "arrowleft" => return Some(Key::LeftArrow),
            "right" | "arrowright" => return Some(Key::RightArrow),

            // F 键
            "f1" => return Some(Key::F1),
            "f2" => return Some(Key::F2),
            "f3" => return Some(Key::F3),
            "f4" => return Some(Key::F4),
            "f5" => return Some(Key::F5),
            "f6" => return Some(Key::F6),
            "f7" => return Some(Key::F7),
            "f8" => return Some(Key::F8),
            "f9" => return Some(Key::F9),
            "f10" => return Some(Key::F10),
            "f11" => return Some(Key::F11),
            "f12" => return Some(Key::F12),

            // 修饰键
            "shift" => return Some(Key::Shift),
            "control" | "ctrl" => return Some(Key::Control),
            "alt" => return Some(Key::Alt),
            "meta" | "command" | "win" => return Some(Key::Meta),

            _ => {}
        }

        // 单字符键
        if key_str.len() == 1 {
            key_str.chars().next().map(Key::Unicode)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_parsing() {
        assert!(InputController::parse_key("enter").is_some());
        assert!(InputController::parse_key("a").is_some());
        assert!(InputController::parse_key("F1").is_some());
        assert!(InputController::parse_key("up").is_some());
    }
}
