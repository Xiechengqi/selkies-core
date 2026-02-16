//! Wayland protocol handlers for the compositor

pub mod compositor;
pub mod xdg_shell;

use crate::compositor::Compositor;

use std::io::Write;
use std::os::fd::OwnedFd;

use smithay::input::dnd::{DnDGrab, DndGrabHandler, GrabType, Source};
use smithay::input::pointer::Focus;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::Serial;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::{
    set_data_device_focus,
    DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler,
};
use smithay::wayland::selection::{SelectionHandler, SelectionSource, SelectionTarget};
use smithay::wayland::text_input::TextInputSeat;
use smithay::wayland::shell::xdg::decoration::XdgDecorationHandler;
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::wayland::shell::xdg::ToplevelSurface;
use smithay::{delegate_data_device, delegate_output, delegate_seat, delegate_text_input_manager, delegate_xdg_decoration};

impl SeatHandler for Compositor {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Compositor> {
        &mut self.seat_state
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        image: smithay::input::pointer::CursorImageStatus,
    ) {
        self.cursor_status = image;
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&WlSurface>) {
        let dh = &self.display_handle;
        let client = focused.and_then(|s| dh.get_client(s.id()).ok());
        set_data_device_focus(dh, seat, client);

        // Update text input focus
        let text_input = seat.text_input();
        text_input.leave();
        if let Some(surface) = focused {
            text_input.set_focus(Some(surface.clone()));
            text_input.enter();
        } else {
            text_input.set_focus(None);
        }

        // Set/unset xdg_toplevel Activated state so clients (e.g. Chromium)
        // know the window has keyboard focus and should process key events.
        for window in self.space.elements() {
            let toplevel = window.toplevel().unwrap();
            let is_focused = focused
                .map(|f| f.id() == toplevel.wl_surface().id())
                .unwrap_or(false);
            toplevel.with_pending_state(|state| {
                if is_focused {
                    state.states.set(xdg_toplevel::State::Activated);
                } else {
                    state.states.unset(xdg_toplevel::State::Activated);
                }
            });
            toplevel.send_pending_configure();
        }
    }
}

delegate_seat!(Compositor);

impl SelectionHandler for Compositor {
    type SelectionUserData = ();

    fn new_selection(&mut self, ty: SelectionTarget, source: Option<SelectionSource>, _seat: Seat<Self>) {
        log::info!("new_selection called: ty={:?}, has_source={}", ty, source.is_some());
        if ty != SelectionTarget::Clipboard {
            return;
        }
        let source = match source {
            Some(s) => s,
            None => {
                log::debug!("new_selection: no source (compositor-owned selection), skipping");
                return;
            }
        };

        // Suppress client re-assertions that happen right after browser→compositor
        // clipboard set. When we call set_data_device_selection, the focused client
        // (e.g. Chromium) re-asserts its own wl_data_source with stale content.
        if let Some(until) = self.clipboard_suppress_until {
            if std::time::Instant::now() < until {
                log::info!("new_selection: suppressed (within browser clipboard window)");
                return;
            }
            self.clipboard_suppress_until = None;
        }

        let mime_types = source.mime_types();
        log::info!("new_selection: mime_types={:?}", mime_types);
        let text_mime = mime_types.iter().find(|m| {
            m.contains("text/plain") || m.contains("UTF8_STRING") || m.contains("utf8")
        });
        let mime = match text_mime {
            Some(m) => m.clone(),
            None => {
                log::warn!("new_selection: no text mime type found in {:?}", mime_types);
                return;
            }
        };

        // Defer the actual data request to the main loop.
        // smithay updates seat_data.clipboard_selection AFTER new_selection returns,
        // so calling request_data_device_client_selection here would fail because
        // the selection is still the old compositor-owned one.
        log::info!("new_selection: deferring clipboard read for mime={}", mime);
        self.clipboard_pending_mime = Some(mime);
    }

    fn send_selection(
        &mut self,
        _ty: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        _user_data: &Self::SelectionUserData,
    ) {
        log::info!("send_selection called: mime={}, has_pending_paste={}", mime_type, self.pending_paste.is_some());
        if let Some(ref text) = self.pending_paste {
            if mime_type.contains("text") || mime_type.contains("string") || mime_type.contains("utf8") {
                let mut file = std::fs::File::from(fd);
                let _ = file.write_all(text.as_bytes());
            }
        }
    }
}

impl DataDeviceHandler for Compositor {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl DndGrabHandler for Compositor {}
impl WaylandDndGrabHandler for Compositor {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        _icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        type_: GrabType,
    ) {
        match type_ {
            GrabType::Pointer => {
                let ptr = seat.get_pointer().unwrap();
                let start_data = ptr.grab_start_data().unwrap();
                let grab = DnDGrab::new_pointer(&self.display_handle, start_data, source, seat);
                ptr.set_grab(self, grab, serial, Focus::Keep);
            }
            GrabType::Touch => {
                source.cancel();
            }
        }
    }
}

delegate_data_device!(Compositor);

impl OutputHandler for Compositor {}
delegate_output!(Compositor);
delegate_text_input_manager!(Compositor);

impl XdgDecorationHandler for Compositor {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        toplevel.send_pending_configure();
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: Mode) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        toplevel.send_pending_configure();
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(Mode::ServerSide);
        });
        toplevel.send_pending_configure();
    }
}
delegate_xdg_decoration!(Compositor);
