//! The state of the window, which is shared with the event-loop.

use std::num::NonZeroU32;
use std::sync::{Arc, Mutex, Weak};

use ahash::HashSet;
use dpi::{LogicalPosition, LogicalSize, PhysicalPosition, PhysicalSize, Size};
use sctk::compositor::{CompositorState, Region, SurfaceData, SurfaceDataExt};
use sctk::reexports::client::backend::ObjectId;
use sctk::reexports::client::protocol::wl_shm::WlShm;
use sctk::reexports::client::{Proxy, QueueHandle};
use sctk::reexports::csd_frame::WindowState as XdgWindowState;
use sctk::reexports::protocols::wp::fractional_scale::v1::client::wp_fractional_scale_v1::WpFractionalScaleV1;
use sctk::reexports::protocols::wp::text_input::zv3::client::zwp_text_input_v3::ZwpTextInputV3;
use sctk::reexports::protocols::wp::viewporter::client::wp_viewport::WpViewport;
use sctk::seat::pointer::{PointerDataExt, ThemedPointer};
use sctk::shell::xdg::window::WindowConfigure;
use sctk::shell::WaylandSurface;
use sctk::shm::slot::SlotPool;
use sctk::shm::Shm;
use sctk::subcompositor::SubcompositorState;
use tracing::{info, warn};
use wayland_protocols_plasma::blur::client::org_kde_kwin_blur::OrgKdeKwinBlur;
use winit_core::cursor::{CursorIcon, CustomCursor as CoreCustomCursor};
use winit_core::error::{NotSupportedError, RequestError};
use winit_core::window::{
    CursorGrabMode, ImeCapabilities, ImeRequest, ImeRequestError, ResizeDirection, Theme,
};
use wayland_protocols_plasma::surface_extension::client::qt_extended_surface::QtExtendedSurface;

use crate::make_wid;
use crate::maliit_ime::MaliitInputMethod;
use crate::event_loop::OwnedDisplayHandle;
use crate::logical_to_physical_rounded;
use crate::seat::{
    PointerConstraintsState, TextInputClientState, WinitPointerData, WinitPointerDataExt,
    ZwpTextInputV3Ext,
};
use crate::state::WinitState;
use crate::types::cursor::{CustomCursor, SelectedCursor, WaylandCustomCursor};
use crate::types::kwin_blur::KWinBlurManager;

use crate::shell::wl_shell::window::{Window as WlShellWindow, WlShellSurfaceResize};

// Minimum window surface size.
const MIN_WINDOW_SIZE: LogicalSize<u32> = LogicalSize::new(2, 1);

/// The state of the window which is being updated from the [`WinitState`].
#[derive(Debug)]
pub struct WindowState {
    /// The connection to Wayland server.
    pub handle: Arc<OwnedDisplayHandle>,

    /// The `Shm` to set cursor.
    pub shm: WlShm,

    /// The last received configure.
    pub last_configure: Option<WindowConfigure>,

    /// The pointers observed on the window.
    pub pointers: Vec<Weak<ThemedPointer<WinitPointerData>>>,

    selected_cursor: SelectedCursor,

    /// Whether the cursor is visible.
    pub cursor_visible: bool,

    /// Pointer constraints to lock/confine pointer.
    pub pointer_constraints: Option<Arc<PointerConstraintsState>>,

    /// Queue handle.
    pub queue_handle: QueueHandle<WinitState>,

    /// Theme variant.
    theme: Option<Theme>,

    /// The current window title.
    title: String,

    /// A shared pool where to allocate images (used for window icons and custom cursors)
    image_pool: Arc<Mutex<SlotPool>>,

    /// Whether the frame is resizable.
    resizable: bool,

    // NOTE: we can't use simple counter, since it's racy when seat getting destroyed and new
    // is created, since add/removed stuff could be delivered a bit out of order.
    /// Seats that has keyboard focus on that window.
    seat_focus: HashSet<ObjectId>,

    /// The scale factor of the window.
    scale_factor: f64,

    /// Whether the window is transparent.
    transparent: bool,

    /// The state of the compositor to create WlRegions.
    compositor: Arc<CompositorState>,

    /// The current cursor grabbing mode.
    cursor_grab_mode: GrabState,

    /// The input method properties provided by the application to the IME.
    ///
    /// This state is cached here so that the window can automatically send the state to the IME as
    /// soon as it becomes available without application involvement.
    text_input_state: Option<TextInputClientState>,

    /// The text inputs observed on the window.
    text_inputs: Vec<ZwpTextInputV3>,

    /// The surface size of the window, as in without client side decorations.
    size: LogicalSize<u32>,

    /// Whether we should decorate the frame.
    decorate: bool,

    /// Min size.
    min_surface_size: LogicalSize<u32>,
    max_surface_size: Option<LogicalSize<u32>>,

    /// The size of the window when no states were applied to it. The primary use for it
    /// is to fallback to original window size, before it was maximized, if the compositor
    /// sends `None` for the new size in the configure.
    stateless_size: LogicalSize<u32>,

    /// Initial window size provided by the user. Removed on the first
    /// configure.
    initial_size: Option<Size>,

    /// The state of the frame callback.
    frame_callback_state: FrameCallbackState,

    viewport: Option<WpViewport>,
    fractional_scale: Option<WpFractionalScaleV1>,
    blur: Option<OrgKdeKwinBlur>,
    blur_manager: Option<KWinBlurManager>,

    /// The underlying WlShell window.
    pub window: WlShellWindow,
    has_focus: bool,
    // QtExtendedSurface global, provides close event
    _extended_surface: Option<QtExtendedSurface>,
    maliit_ime: MaliitInputMethod,
}

impl WindowState {
    /// Create new window state.
    pub fn new(
        handle: Arc<OwnedDisplayHandle>,
        queue_handle: &QueueHandle<WinitState>,
        winit_state: &WinitState,
        initial_size: Size,
        window: WlShellWindow,
        theme: Option<Theme>,
        event_loop_awakener: calloop::ping::Ping,
    ) -> Self {
        let compositor = winit_state.compositor_state.clone();
        let pointer_constraints = winit_state.pointer_constraints.clone();
        let viewport = winit_state
            .viewporter_state
            .as_ref()
            .map(|state| state.get_viewport(window.wl_surface(), queue_handle));
        let fractional_scale = winit_state
            .fractional_scaling_manager
            .as_ref()
            .map(|fsm| fsm.fractional_scaling(window.wl_surface(), queue_handle));

        let extended_surface = winit_state.surface_extension.as_ref()
            .map(|se| se.get_extended_surface(window.wl_surface(), &queue_handle));

        let maliit_ime = MaliitInputMethod::new(make_wid(window.wl_surface()), event_loop_awakener, winit_state.window_events_sink.clone());

        Self {
            blur: None,
            blur_manager: winit_state.kwin_blur_manager.clone(),
            compositor,
            handle,
            cursor_grab_mode: GrabState::new(),
            selected_cursor: Default::default(),
            cursor_visible: true,
            decorate: true,
            fractional_scale,
            frame_callback_state: FrameCallbackState::None,
            seat_focus: Default::default(),
            text_input_state: None,
            last_configure: None,
            max_surface_size: None,
            min_surface_size: MIN_WINDOW_SIZE,
            pointer_constraints,
            pointers: Default::default(),
            queue_handle: queue_handle.clone(),
            resizable: true,
            scale_factor: 1.,
            shm: winit_state.shm.wl_shm().clone(),
            image_pool: winit_state.image_pool.clone(),
            size: initial_size.to_logical(1.),
            stateless_size: initial_size.to_logical(1.),
            initial_size: Some(initial_size),
            text_inputs: Vec::new(),
            theme,
            title: String::default(),
            transparent: false,
            viewport,
            window,
            has_focus: false,
            _extended_surface: extended_surface,
            maliit_ime: maliit_ime,
        }
    }

    /// Apply closure on the given pointer.
    fn apply_on_pointer<F: FnMut(&ThemedPointer<WinitPointerData>, &WinitPointerData)>(
        &self,
        mut callback: F,
    ) {
        self.pointers.iter().filter_map(Weak::upgrade).for_each(|pointer| {
            let data = pointer.pointer().winit_data();
            callback(pointer.as_ref(), data);
        })
    }

    /// Get the current state of the frame callback.
    pub fn frame_callback_state(&self) -> FrameCallbackState {
        self.frame_callback_state
    }

    /// The frame callback was received, but not yet sent to the user.
    pub fn frame_callback_received(&mut self) {
        self.frame_callback_state = FrameCallbackState::Received;
    }

    /// Reset the frame callbacks state.
    pub fn frame_callback_reset(&mut self) {
        self.frame_callback_state = FrameCallbackState::None;
    }

    /// Request a frame callback if we don't have one for this window in flight.
    pub fn request_frame_callback(&mut self) {
        let surface = self.window.wl_surface();
        match self.frame_callback_state {
            FrameCallbackState::None | FrameCallbackState::Received => {
                self.frame_callback_state = FrameCallbackState::Requested;
                surface.frame(&self.queue_handle, surface.clone());
            },
            FrameCallbackState::Requested => (),
        }
    }

    pub fn configure(
        &mut self,
        configure: WindowConfigure,
        _shm: &Shm,
        _subcompositor: &Option<Arc<SubcompositorState>>,
    ) -> bool {
        // NOTE: when using fractional scaling or wl_compositor@v6 the scaling
        // should be delivered before the first configure, thus apply it to
        // properly scale the physical sizes provided by the users.
        if let Some(initial_size) = self.initial_size.take() {
            self.size = initial_size.to_logical(self.scale_factor());
            self.stateless_size = self.size;
        }

        let stateless = Self::is_stateless(&configure);

        let (mut new_size, constrain) = match configure.new_size {
            (Some(width), Some(height)) => ((width.get(), height.get()).into(), false),
            _ if stateless => (self.stateless_size, true),
            _ => (self.size, true),
        };

        // Apply configure bounds only when compositor let the user decide what size to pick.
        if constrain {
            let bounds = self.surface_size_bounds(&configure);
            new_size.width =
                bounds.0.map(|bound_w| new_size.width.min(bound_w.get())).unwrap_or(new_size.width);
            new_size.height = bounds
                .1
                .map(|bound_h| new_size.height.min(bound_h.get()))
                .unwrap_or(new_size.height);
        }

        let new_state = configure.state;
        let old_state = self.last_configure.as_ref().map(|configure| configure.state);

        let state_change_requires_resize = old_state
            .map(|old_state| {
                !old_state
                    .symmetric_difference(new_state)
                    .difference(XdgWindowState::ACTIVATED | XdgWindowState::SUSPENDED)
                    .is_empty()
            })
            // NOTE: `None` is present for the initial configure, thus we must always resize.
            .unwrap_or(true);

        // NOTE: Set the configure before doing a resize, since we query it during it.
        self.last_configure = Some(configure);

        if state_change_requires_resize || new_size != self.surface_size() {
            self.resize(new_size);
            true
        } else {
            false
        }
    }

    /// Compute the bounds for the surface size of the surface.
    fn surface_size_bounds(
        &self,
        configure: &WindowConfigure,
    ) -> (Option<NonZeroU32>, Option<NonZeroU32>) {
        let configure_bounds = match configure.suggested_bounds {
            Some((width, height)) => (NonZeroU32::new(width), NonZeroU32::new(height)),
            None => (None, None),
        };

        configure_bounds
    }

    #[inline]
    fn is_stateless(configure: &WindowConfigure) -> bool {
        !(configure.is_maximized() || configure.is_fullscreen() || configure.is_tiled())
    }

    /// Start interacting drag resize.
    pub fn drag_resize_window(&self, direction: ResizeDirection) -> Result<(), RequestError> {
        let wl_shell_surface = self.window.wl_shell_surface();

        // TODO(kchibisov) handle touch serials.
        self.apply_on_pointer(|_, data| {
            let serial = data.latest_button_serial();
            let seat = data.seat();
            wl_shell_surface.resize(seat, serial, WlShellSurfaceResize::from(direction).into());
        });

        Ok(())
    }

    /// Start the window drag.
    pub fn drag_window(&self) -> Result<(), RequestError> {
        let wl_shell_surface = self.window.wl_shell_surface();
        // TODO(kchibisov) handle touch serials.
        self.apply_on_pointer(|_, data| {
            let serial = data.latest_button_serial();
            let seat = data.seat();
            wl_shell_surface._move(seat, serial);
        });

        Ok(())
    }

    /// Get the stored resizable state.
    #[inline]
    pub fn resizable(&self) -> bool {
        self.resizable
    }

    /// Set the resizable state on the window.
    ///
    /// Returns `true` when the state was applied.
    #[inline]
    pub fn set_resizable(&mut self, resizable: bool) -> bool {
        if self.resizable == resizable {
            return false;
        }

        self.resizable = resizable;
        if resizable {
            // Restore min/max sizes of the window.
            self.reload_min_max_hints();
        } else {
            self.set_min_surface_size(Some(self.size));
            self.set_max_surface_size(Some(self.size));
        }

        true
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.has_focus = focused;
    }

    /// Whether the window is focused by any seat.
    #[inline]
    pub fn has_focus(&self) -> bool {
        self.has_focus
    }

    /// Whether the IME is allowed.
    #[inline]
    pub fn ime_allowed(&self) -> Option<ImeCapabilities> {
        self.text_input_state.as_ref().map(|state| state.capabilities())
    }

    pub(crate) fn text_input_state(&self) -> Option<&TextInputClientState> {
        self.text_input_state.as_ref()
    }

    /// Get the size of the window.
    #[inline]
    pub fn surface_size(&self) -> LogicalSize<u32> {
        self.size
    }

    /// Whether the window received initial configure event from the compositor.
    #[inline]
    pub fn is_configured(&self) -> bool {
        self.last_configure.is_some()
    }

    #[inline]
    pub fn is_decorated(&mut self) -> bool {
        // There is only server side decorations available for wl_shell.
        true
    }

    /// Get the outer size of the window.
    #[inline]
    pub fn outer_size(&self) -> LogicalSize<u32> {
        self.size
    }

    /// Register pointer on the top-level.
    pub fn pointer_entered(&mut self, added: Weak<ThemedPointer<WinitPointerData>>) {
        self.pointers.push(added);
        self.reload_cursor_style();

        let mode = self.cursor_grab_mode.user_grab_mode;
        let _ = self.set_cursor_grab_inner(mode);
    }

    /// Pointer has left the top-level.
    pub fn pointer_left(&mut self, removed: Weak<ThemedPointer<WinitPointerData>>) {
        let mut new_pointers = Vec::new();
        for pointer in self.pointers.drain(..) {
            if let Some(pointer) = pointer.upgrade() {
                if pointer.pointer() != removed.upgrade().unwrap().pointer() {
                    new_pointers.push(Arc::downgrade(&pointer));
                }
            }
        }

        self.pointers = new_pointers;
    }

    /// Refresh the decorations frame if it's present returning whether the client should redraw.
    pub fn refresh_frame(&mut self) -> bool {
        false
    }

    /// Reload the cursor style on the given window.
    pub fn reload_cursor_style(&mut self) {
        if self.cursor_visible {
            match &self.selected_cursor {
                SelectedCursor::Named(icon) => self.set_cursor(*icon),
                SelectedCursor::Custom(cursor) => self.apply_custom_cursor(cursor),
            }
        } else {
            self.set_cursor_visible(self.cursor_visible);
        }
    }

    /// Reissue the transparency hint to the compositor.
    pub fn reload_transparency_hint(&self) {
        let surface = self.window.wl_surface();

        if self.transparent {
            surface.set_opaque_region(None);
        } else if let Ok(region) = Region::new(&*self.compositor) {
            region.add(0, 0, i32::MAX, i32::MAX);
            surface.set_opaque_region(Some(region.wl_region()));
        } else {
            warn!("Failed to mark window opaque.");
        }
    }

    /// Try to resize the window when the user can do so.
    pub fn request_surface_size(&mut self, surface_size: Size) -> PhysicalSize<u32> {
        if self.last_configure.as_ref().map(Self::is_stateless).unwrap_or(true) {
            self.resize(surface_size.to_logical(self.scale_factor()))
        }

        logical_to_physical_rounded(self.surface_size(), self.scale_factor())
    }

    /// Resize the window to the new surface size.
    pub fn resize(&mut self, surface_size: LogicalSize<u32>) {
        self.size = surface_size;

        // Update the stateless size.
        if Some(true) == self.last_configure.as_ref().map(Self::is_stateless) {
            self.stateless_size = surface_size;
        }

        // Reload the hint.
        self.reload_transparency_hint();

        // Set the window geometry.
        // self.window.resize(seat, serial, edges);

        // Update the target viewport, this is used if and only if fractional scaling is in use.
        if let Some(viewport) = self.viewport.as_ref() {
            // Set surface size without the borders.
            viewport.set_destination(self.size.width as _, self.size.height as _);
        }
    }

    /// Get the scale factor of the window.
    #[inline]
    pub fn scale_factor(&self) -> f64 {
        self.scale_factor
    }

    /// Set the cursor icon.
    pub fn set_cursor(&mut self, cursor_icon: CursorIcon) {
        self.selected_cursor = SelectedCursor::Named(cursor_icon);

        if !self.cursor_visible {
            return;
        }

        self.apply_on_pointer(|pointer, _| {
            if pointer.set_cursor(&self.handle.connection, cursor_icon).is_err() {
                warn!("Failed to set cursor to {:?}", cursor_icon);
            }
        })
    }

    /// Set the custom cursor icon.
    pub(crate) fn set_custom_cursor(&mut self, cursor: CoreCustomCursor) {
        let cursor = match cursor.cast_ref::<WaylandCustomCursor>() {
            Some(cursor) => cursor,
            None => {
                tracing::error!("unrecognized cursor passed to Wayland backend");
                return;
            },
        };

        let cursor = {
            let mut pool = self.image_pool.lock().unwrap();
            CustomCursor::new(&mut pool, cursor)
        };

        if self.cursor_visible {
            self.apply_custom_cursor(&cursor);
        }

        self.selected_cursor = SelectedCursor::Custom(cursor);
    }

    fn apply_custom_cursor(&self, cursor: &CustomCursor) {
        self.apply_on_pointer(|pointer, data| {
            let surface = pointer.surface();

            let scale = if let Some(viewport) = data.viewport() {
                let scale = self.scale_factor();
                let size = PhysicalSize::new(cursor.w, cursor.h).to_logical(scale);
                viewport.set_destination(size.width, size.height);
                scale
            } else {
                let scale = surface.data::<SurfaceData>().unwrap().surface_data().scale_factor();
                surface.set_buffer_scale(scale);
                scale as f64
            };

            surface.attach(Some(cursor.buffer.wl_buffer()), 0, 0);
            if surface.version() >= 4 {
                surface.damage_buffer(0, 0, cursor.w, cursor.h);
            } else {
                let size = PhysicalSize::new(cursor.w, cursor.h).to_logical(scale);
                surface.damage(0, 0, size.width, size.height);
            }
            surface.commit();

            let serial = pointer
                .pointer()
                .data::<WinitPointerData>()
                .and_then(|data| data.pointer_data().latest_enter_serial())
                .unwrap();

            let hotspot =
                PhysicalPosition::new(cursor.hotspot_x, cursor.hotspot_y).to_logical(scale);
            pointer.pointer().set_cursor(serial, Some(surface), hotspot.x, hotspot.y);
        });
    }

    /// Set minimum inner window size.
    pub fn set_min_surface_size(&mut self, size: Option<LogicalSize<u32>>) {
        // Ensure that the window has the right minimum size.
        let mut size = size.unwrap_or(MIN_WINDOW_SIZE);
        size.width = size.width.max(MIN_WINDOW_SIZE.width);
        size.height = size.height.max(MIN_WINDOW_SIZE.height);

        self.min_surface_size = size;
        // self.window.set_min_size(Some(size.into()));
    }

    /// Set maximum inner window size.
    pub fn set_max_surface_size(&mut self, size: Option<LogicalSize<u32>>) {
        self.max_surface_size = size;
        // self.window.set_max_size(size.map(Into::into));
    }

    /// Set the CSD theme.
    pub fn set_theme(&mut self, theme: Option<Theme>) {
        self.theme = theme;
    }

    /// The current theme for CSD decorations.
    #[inline]
    pub fn theme(&self) -> Option<Theme> {
        self.theme
    }

    /// Set the cursor grabbing state on the top-level.
    pub fn set_cursor_grab(&mut self, mode: CursorGrabMode) -> Result<(), RequestError> {
        if self.cursor_grab_mode.user_grab_mode == mode {
            return Ok(());
        }

        self.set_cursor_grab_inner(mode)?;
        // Update user grab on success.
        self.cursor_grab_mode.user_grab_mode = mode;
        Ok(())
    }

    /// Reload the hints for minimum and maximum sizes.
    pub fn reload_min_max_hints(&mut self) {
        self.set_min_surface_size(Some(self.min_surface_size));
        self.set_max_surface_size(self.max_surface_size);
    }

    /// Set the grabbing state on the surface.
    fn set_cursor_grab_inner(&mut self, mode: CursorGrabMode) -> Result<(), RequestError> {
        let pointer_constraints = match self.pointer_constraints.as_ref() {
            Some(pointer_constraints) => pointer_constraints,
            None if mode == CursorGrabMode::None => return Ok(()),
            None => {
                return Err(
                    NotSupportedError::new("zwp_pointer_constraints is not available").into()
                )
            },
        };

        let mut unset_old = false;
        match self.cursor_grab_mode.current_grab_mode {
            CursorGrabMode::None => unset_old = true,
            CursorGrabMode::Confined => self.apply_on_pointer(|_, data| {
                data.unconfine_pointer();
                unset_old = true;
            }),
            CursorGrabMode::Locked => {
                self.apply_on_pointer(|_, data| {
                    data.unlock_pointer();
                    unset_old = true;
                });
            },
        }

        // In case we haven't unset the old mode, it means that we don't have a cursor above
        // the window, thus just wait for it to re-appear.
        if !unset_old {
            return Ok(());
        }

        let mut set_mode = false;
        let surface = self.window.wl_surface();
        match mode {
            CursorGrabMode::Locked => self.apply_on_pointer(|pointer, data| {
                let pointer = pointer.pointer();
                data.lock_pointer(pointer_constraints, surface, pointer, &self.queue_handle);
                set_mode = true;
            }),
            CursorGrabMode::Confined => self.apply_on_pointer(|pointer, data| {
                let pointer = pointer.pointer();
                data.confine_pointer(pointer_constraints, surface, pointer, &self.queue_handle);
                set_mode = true;
            }),
            CursorGrabMode::None => {
                // Current lock/confine was already removed.
                set_mode = true;
            },
        }

        // Replace the current grab mode after we've ensure that it got updated.
        if set_mode {
            self.cursor_grab_mode.current_grab_mode = mode;
        }

        Ok(())
    }

    pub fn show_window_menu(&self, _position: LogicalPosition<u32>) {
        // TODO(kchibisov) handle touch serials.
        // self.apply_on_pointer(|_, data| {
        //     let serial = data.latest_button_serial();
        //     let seat = data.seat();
        //     self.window.show_window_menu(seat, serial, position.into());
        // });
    }

    /// Set the position of the cursor.
    pub fn set_cursor_position(&self, position: LogicalPosition<f64>) -> Result<(), RequestError> {
        if self.pointer_constraints.is_none() {
            return Err(NotSupportedError::new("zwp_pointer_constraints is not available").into());
        }

        // Position can be set only for locked cursor.
        if self.cursor_grab_mode.current_grab_mode != CursorGrabMode::Locked {
            return Err(NotSupportedError::new(
                "cursor position could only be changed for locked pointer",
            )
            .into());
        }

        self.apply_on_pointer(|_, data| {
            data.set_locked_cursor_position(position.x, position.y);
        });

        Ok(())
    }

    /// Set the visibility state of the cursor.
    pub fn set_cursor_visible(&mut self, cursor_visible: bool) {
        self.cursor_visible = cursor_visible;

        if self.cursor_visible {
            match &self.selected_cursor {
                SelectedCursor::Named(icon) => self.set_cursor(*icon),
                SelectedCursor::Custom(cursor) => self.apply_custom_cursor(cursor),
            }
        } else {
            for pointer in self.pointers.iter().filter_map(|pointer| pointer.upgrade()) {
                let latest_enter_serial = pointer.pointer().winit_data().latest_enter_serial();

                pointer.pointer().set_cursor(latest_enter_serial, None, 0, 0);
            }
        }
    }

    /// Whether show or hide client side decorations.
    #[inline]
    pub fn set_decorate(&mut self, decorate: bool) {
        if decorate == self.decorate {
            return;
        }

        self.decorate = decorate;
    }

    /// Add seat focus for the window.
    #[inline]
    pub fn add_seat_focus(&mut self, seat: ObjectId) {
        self.seat_focus.insert(seat);
    }

    /// Remove seat focus from the window.
    #[inline]
    pub fn remove_seat_focus(&mut self, seat: &ObjectId) {
        self.seat_focus.remove(seat);
    }

    /// Atomically update input method state.
    ///
    /// Returns `None` if an input method state haven't changed. Alternatively `Some(true)` and
    /// `Some(false)` is returned respectfully.
    pub fn request_ime_update(
        &mut self,
        request: ImeRequest,
    ) -> Result<Option<bool>, ImeRequestError> {
        let state_change = match request {
            ImeRequest::Enable(enable) => {
                let (capabilities, request_data) = enable.into_raw();

                if self.text_input_state.is_some() {
                    return Err(ImeRequestError::AlreadyEnabled);
                } else {
                    self.maliit_ime.show();
                }

                self.text_input_state = Some(TextInputClientState::new(
                    capabilities,
                    request_data,
                    self.scale_factor(),
                ));
                true
            },
            ImeRequest::Update(request_data) => {
                let scale_factor = self.scale_factor();
                if let Some(text_input_state) = self.text_input_state.as_mut() {
                    text_input_state.update(request_data, scale_factor);
                } else {
                    return Err(ImeRequestError::NotEnabled);
                }
                false
            },
            ImeRequest::Disable => {
                self.maliit_ime.hide();
                self.text_input_state = None;
                true
            },
        };

        // Only one input method may be active per (seat, surface),
        // but there may be multiple seats focused on a surface,
        // resulting in multiple text input objects.
        //
        // WARNING: this doesn't actually handle different seats with independent cursors. There's
        // no API to set a per-seat input method state, so they all share a single state.
        for text_input in &self.text_inputs {
            text_input.set_state(self.text_input_state.as_ref(), state_change);
        }

        if state_change {
            Ok(Some(self.text_input_state.is_some()))
        } else {
            Ok(None)
        }
    }

    /// Set the scale factor for the given window.
    #[inline]
    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = scale_factor;

        // NOTE: When fractional scaling is not used update the buffer scale.
        if self.fractional_scale.is_none() {
            let _ = self.window.set_buffer_scale(self.scale_factor as _);
        }
    }

    /// Make window background blurred
    #[inline]
    pub fn set_blur(&mut self, blurred: bool) {
        if blurred && self.blur.is_none() {
            if let Some(blur_manager) = self.blur_manager.as_ref() {
                let blur = blur_manager.blur(self.window.wl_surface(), &self.queue_handle);
                blur.commit();
                self.blur = Some(blur);
            } else {
                info!("Blur manager unavailable, unable to change blur")
            }
        } else if !blurred && self.blur.is_some() {
            self.blur_manager.as_ref().unwrap().unset(self.window.wl_surface());
            self.blur.take().unwrap().release();
        }
    }

    /// Set the window title to a new value.
    ///
    /// This will automatically truncate the title to something meaningful.
    pub fn set_title(&mut self, mut title: String) {
        // Truncate the title to at most 1024 bytes, so that it does not blow up the protocol
        // messages
        if title.len() > 1024 {
            let mut new_len = 1024;
            while !title.is_char_boundary(new_len) {
                new_len -= 1;
            }
            title.truncate(new_len);
        }

        self.window.set_title(&title);
        self.title = title;
    }

    /// Set the window's icon
    pub fn set_window_icon(&mut self, _window_icon: Option<winit_core::icon::Icon>) {
        // Not supported on AuroraOS
    }

    /// Mark the window as transparent.
    #[inline]
    pub fn set_transparent(&mut self, transparent: bool) {
        self.transparent = transparent;
        self.reload_transparency_hint();
    }

    /// Register text input on the top-level.
    #[inline]
    pub fn text_input_entered(&mut self, text_input: &ZwpTextInputV3) {
        if !self.text_inputs.iter().any(|t| t == text_input) {
            self.text_inputs.push(text_input.clone());
        }
    }

    /// The text input left the top-level.
    #[inline]
    pub fn text_input_left(&mut self, text_input: &ZwpTextInputV3) {
        if let Some(position) = self.text_inputs.iter().position(|t| t == text_input) {
            self.text_inputs.remove(position);
        }
    }

    /// Get the cached title.
    #[inline]
    pub fn title(&self) -> &str {
        &self.title
    }
}

impl Drop for WindowState {
    fn drop(&mut self) {
        if let Some(blur) = self.blur.take() {
            blur.release();
        }

        if let Some(fs) = self.fractional_scale.take() {
            fs.destroy();
        }

        if let Some(viewport) = self.viewport.take() {
            viewport.destroy();
        }

        // NOTE: the wl_surface used by the window is being cleaned up when
        // dropping SCTK `Window`.
    }
}

/// The state of the cursor grabs.
#[derive(Clone, Copy, Debug)]
struct GrabState {
    /// The grab mode requested by the user.
    user_grab_mode: CursorGrabMode,

    /// The current grab mode.
    current_grab_mode: CursorGrabMode,
}

impl GrabState {
    fn new() -> Self {
        Self { user_grab_mode: CursorGrabMode::None, current_grab_mode: CursorGrabMode::None }
    }
}

/// The state of the frame callback.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameCallbackState {
    /// No frame callback was requested.
    #[default]
    None,
    /// The frame callback was requested, but not yet arrived, the redraw events are throttled.
    Requested,
    /// The callback was marked as done, and user could receive redraw requested
    Received,
}
