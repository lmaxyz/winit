use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex, RwLock};
use crate::{event::{KeyEvent, WindowEvent}, platform_impl::wayland::window::WindowState};

use super::{event_loop::sink::EventSink, DeviceId, WindowId};

use dpi::LogicalSize;
use maliit::input_method::InputMethod;


pub struct MaliitInputMethod {
    input_method: Arc<Mutex<InputMethod>>,
    window_id: WindowId,
    window_state: Arc<Mutex<WindowState>>,
    events_sink: Arc<Mutex<EventSink>>,
    event_loop_awakener: calloop::ping::Ping,
    is_events_handling_enabled: Arc<AtomicBool>,
    size: Arc<RwLock<LogicalSize<u32>>>,
}

impl MaliitInputMethod {
    pub fn new(window_id: WindowId, window_state: Arc<Mutex<WindowState>>, event_loop_awakener: calloop::ping::Ping, events_sink: Arc<Mutex<EventSink>>) -> Self {
        Self {
            window_id,
            window_state,
            events_sink,
            event_loop_awakener,
            input_method: Arc::new(Mutex::new(InputMethod::new().unwrap())),
            is_events_handling_enabled: Arc::new(AtomicBool::new(false)),
            size: Default::default()
        }
    }

    pub fn show(&self) {
        if !self.is_events_handling_enabled.load(Ordering::Relaxed) {
            {
                let mut im = self.input_method.lock().unwrap();
                im.show();
            }
            self.start_events_handling();
            self.events_sink.lock().unwrap().push_window_event(WindowEvent::RedrawRequested, self.window_id);
            while self.size().height == 0 {
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
        }
    }

    pub fn hide(&self) {
        self.is_events_handling_enabled.store(false, Ordering::Relaxed);
        if let Ok(mut window_state) = self.window_state.lock() {
            let mut new_size = window_state.inner_size();
            new_size.height += self.size.read().unwrap().height as u32;
            window_state.resize(new_size);
            // Меняем размер здесь, потому что не успеваем получить событие
            // let mut size = self.size.write().unwrap();
            // (*size).height = 0;
            // (*size).width = 0;
        }
        let mut im = self.input_method.lock().unwrap();
        im.hide();
        im.poll_new_events(std::time::Duration::from_millis(30)); // Skip accumulated events
    }

    pub fn size(&self) -> LogicalSize<u32> {
        self.size.read().unwrap().to_owned()
    }

    fn start_events_handling(&self) {
        let events_sink = self.events_sink.clone();
        let window_id = self.window_id;
        let event_loop_awakener = self.event_loop_awakener.clone();
        let is_events_handling_enabled = self.is_events_handling_enabled.clone();
        let input_method = self.input_method.clone();
        let ime_size = self.size.clone();
        let window_state = self.window_state.clone();
        is_events_handling_enabled.store(true, Ordering::Relaxed);

        std::thread::spawn(move || {
            loop {
                if let Some(events) = input_method.lock().unwrap().poll_new_events(std::time::Duration::from_millis(30)) {
                    for event in events {
                        let mut events_sink = events_sink.lock().unwrap();
                        match event {
                            maliit::events::InputMethodEvent::Text(txt) => {
                                if txt.chars().count() == 1 {
                                    events_sink.push_window_event(kb_input_event_from_str(&txt), window_id);
                                } else {
                                    let preedit_event = WindowEvent::Ime(crate::event::Ime::Preedit((&txt).into(), Some((0, txt.len()))));
                                    let commit_event = WindowEvent::Ime(crate::event::Ime::Commit(txt.into()));
                                    events_sink.push_window_event(preedit_event, window_id);
                                    events_sink.push_window_event(commit_event, window_id);
                                }
                            },
                            maliit::events::InputMethodEvent::Key { key, pressed } => {
                                events_sink.push_window_event(kb_input_event_from_key(key, pressed), window_id);
                            },
                            maliit::events::InputMethodEvent::AreaChanged(_x, y, width, height) => {
                                if y == 0 {
                                    is_events_handling_enabled.store(false, Ordering::Relaxed);
                                } else if let Ok(mut ime_size) = ime_size.write() {
                                    (*ime_size).height = height as u32;
                                    (*ime_size).width = width as u32;
                                }
                                if let Ok(mut window_state) = window_state.lock() {
                                    let mut new_size = window_state.inner_size();
                                    if y == 0 {
                                        new_size.height += ime_size.read().unwrap().height as u32;
                                    } else {
                                        new_size.height -= ime_size.read().unwrap().height as u32;
                                    }

                                    window_state.resize(new_size);
                                }
                            }
                        };
                    }
                    event_loop_awakener.ping();
                }

                if !is_events_handling_enabled.load(Ordering::Relaxed) {
                    events_sink.lock().unwrap().push_window_event(WindowEvent::Ime(crate::event::Ime::Disabled), window_id);
                    break
                }
            }
        });
    }
}

fn kb_input_event_from_str(input: &str) -> WindowEvent {
    let device_id = crate::event::DeviceId(crate::platform_impl::DeviceId::Wayland(DeviceId));
    let key_event = KeyEvent {
        physical_key:  crate::keyboard::PhysicalKey::Unidentified(crate::keyboard::NativeKeyCode::Unidentified),
        logical_key: crate::keyboard::Key::Character(input.into()),
        text: Some(input.into()),
        repeat: false,
        state: crate::event::ElementState::Pressed,
        location: crate::keyboard::KeyLocation::Standard,
        platform_specific: crate::platform_impl::KeyEventExtra { text_with_all_modifiers: None, key_without_modifiers: crate::keyboard::Key::Dead(None) }
    };
    WindowEvent::KeyboardInput { device_id: device_id, event: key_event, is_synthetic: false }
}

fn kb_input_event_from_key(key: maliit::events::Key, pressed: bool) -> WindowEvent {
    let device_id = crate::event::DeviceId(crate::platform_impl::DeviceId::Wayland(DeviceId));
    let key_event = KeyEvent {
        physical_key:  crate::keyboard::PhysicalKey::Code(maliit_key_to_key_code(key)),
        logical_key: crate::keyboard::Key::Named(maliit_key_to_named_key(key)),
        text: None,
        repeat: false,
        state: if pressed { crate::event::ElementState::Pressed } else { crate::event::ElementState::Released },
        location: crate::keyboard::KeyLocation::Standard,
        platform_specific: crate::platform_impl::KeyEventExtra { text_with_all_modifiers: None, key_without_modifiers: crate::keyboard::Key::Dead(None) }
    };
    WindowEvent::KeyboardInput { device_id: device_id, event: key_event, is_synthetic: false }
}

fn maliit_key_to_named_key(key: maliit::events::Key) -> crate::keyboard::NamedKey {
    match key {
        maliit::events::Key::Enter => crate::keyboard::NamedKey::Enter,
        maliit::events::Key::Backspace => crate::keyboard::NamedKey::Backspace
    }
}

fn maliit_key_to_key_code(key: maliit::events::Key) -> crate::keyboard::KeyCode {
    match key {
        maliit::events::Key::Enter => crate::keyboard::KeyCode::Enter,
        maliit::events::Key::Backspace => crate::keyboard::KeyCode::Backspace
    }
}
