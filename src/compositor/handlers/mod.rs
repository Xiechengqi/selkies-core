//! Wayland protocol handlers for the compositor

pub mod compositor;
pub mod xdg_shell;

use crate::compositor::Compositor;

use std::io::Write;
use std::os::fd::{FromRawFd, OwnedFd};

use smithay::input::dnd::{DnDGrab, DndGrabHandler, GrabType, Source};
use smithay::input::pointer::Focus;
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::Resource;
use smithay::utils::Serial;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::selection::data_device::{
    set_data_device_focus, request_data_device_client_selection,
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

    fn new_selection(&mut self, ty: SelectionTarget, source: Option<SelectionSource>, seat: Seat<Self>) {
        if ty != SelectionTarget::Clipboard {
            return;
        }
        let source = match source {
            Some(s) => s,
            None => return,
        };
        let mime_types = source.mime_types();
        let text_mime = mime_types.iter().find(|m| {
            m.contains("text/plain") || m.contains("UTF8_STRING") || m.contains("utf8")
        });
        let mime = match text_mime {
            Some(m) => m.clone(),
            None => return,
        };

        // Create pipe: write_fd goes to client, read_fd we keep
        let mut fds = [0i32; 2];
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return;
        }
        let read_fd = unsafe { OwnedFd::from_raw_fd(fds[0]) };
        let write_fd = unsafe { OwnedFd::from_raw_fd(fds[1]) };

        if request_data_device_client_selection::<Self>(&seat, mime, write_fd).is_ok() {
            // Store read end; main loop will poll it after dispatch
            self.clipboard_read_fd = Some(read_fd);
        }
    }

    fn send_selection(
        &mut self,
        _ty: SelectionTarget,
        mime_type: String,
        fd: OwnedFd,
        _seat: Seat<Self>,
        _user_data: &Self::SelectionUserData,
    ) {
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
