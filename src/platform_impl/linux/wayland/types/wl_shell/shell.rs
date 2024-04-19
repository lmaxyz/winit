use wayland_client::{
    delegate_dispatch, globals::{BindError, GlobalList}, protocol::{wl_shell::{self, WlShell}, wl_shell_surface, wl_surface::WlSurface}, Connection, Dispatch, Proxy, QueueHandle
};

use sctk::{
    error::GlobalError, globals::{GlobalData, ProvidesBoundGlobal}
};

use crate::platform_impl::wayland::state::WinitState;

use super::window::{WlShellWindow, WlWindowHandler};

#[derive(Debug)]
pub struct Shell {
    wl_shell: wl_shell::WlShell,
    
}

impl Shell {
    pub fn bind<State>(globals: &GlobalList, qh: &QueueHandle<State>) -> Result<Shell, BindError>
    where
        State: Dispatch<wl_shell::WlShell, GlobalData, State>  + 'static,
    {
        let wl_shell = globals.bind(qh, 1..=1, GlobalData)?;
        
        Ok(Shell { wl_shell})
    }

    pub fn create_window<State>(&self, surface: WlSurface, qh: &QueueHandle<State>) -> WlShellWindow 
    where
        State: Dispatch<wl_shell_surface::WlShellSurface, GlobalData, State> + 'static
    {
        let wl_shell_surface = self.wl_shell.get_shell_surface(&surface, qh, GlobalData);

        WlShellWindow::new(surface, wl_shell_surface)
    }

    pub fn wl_shell(&self) -> &wl_shell::WlShell {
        &self.wl_shell
    }
}


impl ProvidesBoundGlobal<wl_shell::WlShell, 1> for Shell {
    fn bound_global(&self) -> Result<wl_shell::WlShell, GlobalError> {
        Ok(self.wl_shell.clone())
    }
}

impl Dispatch<WlShell, GlobalData, WinitState> for Shell {
    fn event(
        _state: &mut WinitState,
        _proxy: &WlShell,
        event: <WlShell as Proxy>::Event,
        _data: &GlobalData,
        _conn: &Connection,
        _qhandle: &QueueHandle<WinitState>,
    ) {
        match event {
            _ => {
                log::debug!("Some event was arrived!!!")
            },
        }
    }
}

impl<D> Dispatch<wl_shell_surface::WlShellSurface, GlobalData, D> for Shell
where
    D: Dispatch<wl_shell_surface::WlShellSurface, GlobalData> + WlWindowHandler, {
    fn event(
        _state: &mut D,
        proxy: &wl_shell_surface::WlShellSurface,
        event: wl_shell_surface::Event,
        _data: &GlobalData,
        _conn: &Connection,
        _qhandle: &QueueHandle<D>,
    ) {
        match event {
            wl_shell_surface::Event::Ping { serial } => {
                proxy.pong(serial);
            }
            wl_shell_surface::Event::Configure { edges, width, height } => {
                log::debug!("Configure event was arrived: {:?}, {}, {}", edges, width, height);
            },
            wl_shell_surface::Event::PopupDone => todo!(),
            _ => unreachable!(),
        }
    }

    fn event_created_child(opcode: u16, _qhandle: &QueueHandle<D>) -> std::sync::Arc<dyn wayland_backend::client::ObjectData> {
        panic!(
            "Missing event_created_child specialization for event opcode {} of {}",
            opcode,
            wl_shell_surface::WlShellSurface::interface().name
        );
    }
}

delegate_dispatch!(WinitState: [ wl_shell_surface::WlShellSurface: GlobalData] => Shell);
// delegate_dispatch!(WinitState: [ WlSurface: GlobalData] => Shell);
delegate_dispatch!(WinitState: [ WlShell: GlobalData] => Shell);