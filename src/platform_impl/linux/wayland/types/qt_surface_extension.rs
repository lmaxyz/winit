use sctk::reexports::client::{delegate_dispatch, Connection, Proxy, QueueHandle};
use sctk::reexports::client::globals::{BindError, GlobalList};
use sctk::error::GlobalError;
use sctk::reexports::client::Dispatch;
use sctk::globals::{GlobalData, ProvidesBoundGlobal};
use crate::platform_impl::wayland::shell::wl_shell::window::WindowHandler;

use wayland_client::protocol::wl_surface::WlSurface;
use wayland_protocols_plasma::surface_extension::client::{
    qt_extended_surface::QtExtendedSurface, qt_surface_extension::QtSurfaceExtension,
    qt_extended_surface::Event as ExtendedSurfaceEvent
};

use crate::platform_impl::wayland::state::WinitState;

#[derive(Clone)]
pub struct SurfaceData(WlSurface);

pub struct SurfaceExtension {
    surface_extension: QtSurfaceExtension
}

impl SurfaceExtension {
    pub fn new<State>(globals: &GlobalList, queue_handle: &QueueHandle<State>) -> Result<Self, BindError> 
    where
        State: Dispatch<QtSurfaceExtension, GlobalData, State>  + 'static {
        let surface_extension = globals.bind(queue_handle, 1..=1, GlobalData)?;
        Ok(Self { surface_extension })
    }

    pub fn get_extended_surface<State>(&self, surface: &WlSurface, queue_handle: &QueueHandle<State>) -> QtExtendedSurface
    where
        State: Dispatch<QtExtendedSurface, SurfaceData, State>  + 'static
    {
        
        self.surface_extension.get_extended_surface(surface, queue_handle, SurfaceData(surface.clone()))
    }
}

impl ProvidesBoundGlobal<QtSurfaceExtension, 1> for SurfaceExtension {
    fn bound_global(&self) -> Result<QtSurfaceExtension, GlobalError> {
        Ok(self.surface_extension.clone())
    }
}

impl Dispatch<QtSurfaceExtension, GlobalData, WinitState> for SurfaceExtension {
    fn event(
        _: &mut WinitState,
        _: &QtSurfaceExtension,
        _: <QtSurfaceExtension as Proxy>::Event,
        _: &GlobalData,
        _: &Connection,
        _: &QueueHandle<WinitState>,
    ) {
        unreachable!("no events defined for qt_surface_extension");
    }
}

impl<D> Dispatch<QtExtendedSurface, SurfaceData, D> for SurfaceExtension
    where D: Dispatch<QtExtendedSurface, SurfaceData> + WindowHandler,
{
    fn event(
        state: &mut D,
        _: &QtExtendedSurface,
        event: ExtendedSurfaceEvent,
        data: &SurfaceData,
        conn: &Connection,
        qh: &QueueHandle<D>,
    ) {
        match event {
            ExtendedSurfaceEvent::Close => {
                debug!("CLOSE EVENT!!!");
                state.request_close(conn, qh, &data.0);
            }
            ExtendedSurfaceEvent::OnscreenVisibility { visible } => {
                debug!("VISIBLE EVENT: {}", visible);
            },
            ExtendedSurfaceEvent::SetGenericProperty { name: _, value: _ } => {
                debug!("SetGenericProperty EVENT!!!");
            },
            _ => unreachable!(),
        }
    }
}

delegate_dispatch!(WinitState: [QtSurfaceExtension: GlobalData] => SurfaceExtension);
delegate_dispatch!(WinitState: [QtExtendedSurface: SurfaceData] => SurfaceExtension);
