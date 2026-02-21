//! XDG Shell handler for toplevel windows and popups

use smithay::{
    delegate_xdg_shell,
    desktop::{
        find_popup_root_surface, get_popup_toplevel_coords, PopupKind, PopupManager, Space, Window,
        PopupKeyboardGrab, PopupPointerGrab, PopupUngrabStrategy,
    },
    input::{
        pointer::{Focus, GrabStartData as PointerGrabStartData},
        Seat,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::{
            protocol::{wl_output::WlOutput, wl_seat, wl_surface::WlSurface},
            Resource,
        },
    },
    utils::{Rectangle, Serial},
    wayland::{
        compositor::with_states,
        shell::xdg::{
            PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
            XdgToplevelSurfaceData,
        },
    },
};

use crate::compositor::{
    grabs::{MoveSurfaceGrab, ResizeSurfaceGrab},
    Compositor,
};

/// Check if `child` is a descendant process of `ancestor` via /proc ppid chain.
fn is_descendant_of(child: i32, ancestor: i32) -> bool {
    let mut pid = child;
    for _ in 0..32 {
        if pid <= 1 { return false; }
        if pid == ancestor { return true; }
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{}/stat", pid)) else { return false };
        // Field 4 in /proc/PID/stat is PPID
        let ppid: i32 = stat.split_whitespace().nth(3).and_then(|s| s.parse().ok()).unwrap_or(0);
        pid = ppid;
    }
    false
}

impl XdgShellHandler for Compositor {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let has_parent = surface.parent().is_some();

        // Detect dialog: has explicit parent, same Wayland client, or the new
        // window's process is a child/parent of an existing window's process
        // (handles Chrome subprocess file choosers that are different clients).
        let new_surface_id = surface.wl_surface().id();
        let same_client_toplevel_exists = self.space.elements().any(|w| {
            let existing_id = w.toplevel().unwrap().wl_surface().id();
            existing_id.same_client_as(&new_surface_id)
        });

        let is_child_process = if !same_client_toplevel_exists {
            // Check if new window's PID is a descendant of any existing window's PID
            let new_pid = surface.wl_surface().client()
                .and_then(|c| c.get_credentials(&self.display_handle).ok())
                .map(|c| c.pid);
            if let Some(new_pid) = new_pid {
                self.space.elements().any(|w| {
                    let existing_pid = w.toplevel().unwrap().wl_surface().client()
                        .and_then(|c| c.get_credentials(&self.display_handle).ok())
                        .map(|c| c.pid);
                    if let Some(ep) = existing_pid {
                        is_descendant_of(new_pid, ep) || is_descendant_of(ep, new_pid)
                    } else {
                        false
                    }
                })
            } else {
                false
            }
        } else {
            false
        };

        let is_dialog = has_parent || same_client_toplevel_exists || is_child_process;

        log::info!("new_toplevel: is_dialog={} (parent={}, same_client={}, child_proc={})",
            is_dialog, has_parent, same_client_toplevel_exists, is_child_process);

        let window = Window::new_wayland_window(surface.clone());

        // Extract output geometry before mutably borrowing space
        let output_geo = self.space.outputs().next()
            .and_then(|o| self.space.output_geometry(o));

        self.space.map_element(window, (0, 0), false);

        // Main window (not dialog): set fullscreen to fill the screen.
        // Dialogs/popups: don't force any size - let the app decide.
        if !is_dialog {
            if let Some(output_geo) = output_geo {
                surface.with_pending_state(|state| {
                    state.states.set(xdg_toplevel::State::Fullscreen);
                    state.size = Some((output_geo.size.w, output_geo.size.h).into());
                });
                surface.send_pending_configure();
            }
        }

        // Remember which surfaces are dialogs (for commit handler centering)
        if is_dialog {
            self.dialog_surfaces.insert(surface.wl_surface().id().protocol_id());
        }

        // Auto-focus the new window so it receives keyboard input
        let keyboard = self.seat.get_keyboard().unwrap();
        let focus_serial = smithay::utils::SERIAL_COUNTER.next_serial();
        keyboard.set_focus(self, Some(surface.wl_surface().clone()), focus_serial);

        // Register all non-dialog windows in the taskbar
        if !is_dialog {
            let wl = surface.wl_surface().clone();
            if !self.window_registry.iter().any(|w| w.id() == wl.id()) {
                self.window_registry.push(wl);
                self.taskbar_dirty = true;
            }
        }
    }

    fn parent_changed(&mut self, surface: ToplevelSurface) {
        // When a toplevel gets a parent (becomes a dialog), just track it.
        // Don't force any size - let the app decide.
        if surface.parent().is_some() {
            log::info!("parent_changed: dialog detected");
            self.dialog_surfaces.insert(surface.wl_surface().id().protocol_id());
        }
    }

    fn new_popup(&mut self, surface: PopupSurface, _positioner: PositionerState) {
        self.unconstrain_popup(&surface);
        let _ = self.popups.track_popup(PopupKind::Xdg(surface));
    }

    fn reposition_request(&mut self, surface: PopupSurface, positioner: PositionerState, token: u32) {
        surface.with_pending_state(|state| {
            let geometry = positioner.get_geometry();
            state.geometry = geometry;
            state.positioner = positioner;
        });
        self.unconstrain_popup(&surface);
        surface.send_repositioned(token);
    }

    fn move_request(&mut self, surface: ToplevelSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat = Seat::from_resource(&seat).unwrap();
        let wl_surface = surface.wl_surface();

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();
            let window = self
                .space
                .elements()
                .find(|w| w.toplevel().unwrap().wl_surface() == wl_surface)
                .unwrap()
                .clone();
            let initial_window_location = self.space.element_location(&window).unwrap();

            let grab = MoveSurfaceGrab {
                start_data,
                window,
                initial_window_location,
            };
            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn resize_request(
        &mut self,
        surface: ToplevelSurface,
        seat: wl_seat::WlSeat,
        serial: Serial,
        edges: xdg_toplevel::ResizeEdge,
    ) {
        let seat = Seat::from_resource(&seat).unwrap();
        let wl_surface = surface.wl_surface();

        if let Some(start_data) = check_grab(&seat, wl_surface, serial) {
            let pointer = seat.get_pointer().unwrap();
            let window = self
                .space
                .elements()
                .find(|w| w.toplevel().unwrap().wl_surface() == wl_surface)
                .unwrap()
                .clone();
            let initial_window_location = self.space.element_location(&window).unwrap();
            let initial_window_size = window.geometry().size;

            surface.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Resizing);
            });
            surface.send_pending_configure();

            let grab = ResizeSurfaceGrab::start(
                start_data,
                window,
                edges.into(),
                Rectangle::new(initial_window_location, initial_window_size),
            );
            pointer.set_grab(self, grab, serial, Focus::Clear);
        }
    }

    fn maximize_request(&mut self, surface: ToplevelSurface) {
        // Don't maximize dialogs — they should stay as floating windows
        let sid = surface.wl_surface().id().protocol_id();
        if self.dialog_surfaces.contains(&sid) {
            return;
        }

        let wl_surface = surface.wl_surface().clone();
        let output = self.space.outputs().next().unwrap().clone();
        let output_geo = self.space.output_geometry(&output).unwrap();

        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Fullscreen);
            state.size = Some((output_geo.size.w, output_geo.size.h).into());
        });
        surface.send_pending_configure();

        let window = self.space.elements()
            .find(|w| w.toplevel().unwrap().wl_surface() == &wl_surface)
            .cloned();
        if let Some(window) = window {
            self.space.map_element(window, (0, 0), true);
        }
    }

    fn unmaximize_request(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Maximized);
            state.states.unset(xdg_toplevel::State::Fullscreen);
            state.size = None;
        });
        surface.send_pending_configure();
    }

    fn fullscreen_request(&mut self, surface: ToplevelSurface, _output: Option<WlOutput>) {
        let sid = surface.wl_surface().id().protocol_id();
        if self.dialog_surfaces.contains(&sid) {
            return;
        }

        let wl_surface = surface.wl_surface().clone();
        let output = self.space.outputs().next().unwrap().clone();
        let output_geo = self.space.output_geometry(&output).unwrap();

        surface.with_pending_state(|state| {
            state.states.set(xdg_toplevel::State::Fullscreen);
            state.size = Some((output_geo.size.w, output_geo.size.h).into());
        });
        surface.send_pending_configure();

        let window = self.space.elements()
            .find(|w| w.toplevel().unwrap().wl_surface() == &wl_surface)
            .cloned();
        if let Some(window) = window {
            self.space.map_element(window, (0, 0), true);
        }
    }

    fn unfullscreen_request(&mut self, surface: ToplevelSurface) {
        surface.with_pending_state(|state| {
            state.states.unset(xdg_toplevel::State::Fullscreen);
            state.size = None;
        });
        surface.send_pending_configure();
    }

    fn minimize_request(&mut self, surface: ToplevelSurface) {
        let wl_surface = surface.wl_surface().clone();
        let window = self.space.elements()
            .find(|w| w.toplevel().unwrap().wl_surface() == &wl_surface)
            .cloned();
        if let Some(window) = window {
            self.space.unmap_elem(&window);
        }
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        self.taskbar_dirty = true;

        let proto_id = surface.wl_surface().id().protocol_id();
        let is_dialog = self.dialog_surfaces.remove(&proto_id);

        // Remove only the destroyed surface from window registry (not siblings)
        let surf_id = surface.wl_surface().id();
        self.window_registry.retain(|wl| wl.id() != surf_id);

        // Only kill the owning process if it's not a dialog and no other toplevels
        // from the same client remain. Dialog processes (file choosers, etc.) may be
        // child processes whose termination cascades to the parent app.
        let has_sibling = self.space.elements().any(|w| {
            let wl_id = w.toplevel().unwrap().wl_surface().id();
            wl_id != surf_id && wl_id.same_client_as(&surf_id)
        });
        if !is_dialog && !has_sibling {
            if let Some(client) = surface.wl_surface().client() {
                if let Ok(creds) = client.get_credentials(&self.display_handle) {
                    let pid = creds.pid;
                    if pid > 1 {
                        log::info!("Killing client process (pid={}) for destroyed toplevel", pid);
                        unsafe { libc::kill(pid, libc::SIGTERM); }
                    }
                }
            }
        }
    }

    fn grab(&mut self, surface: PopupSurface, seat: wl_seat::WlSeat, serial: Serial) {
        let seat: Seat<Compositor> = Seat::from_resource(&seat).unwrap();
        let kind = PopupKind::Xdg(surface);

        // Find the root toplevel surface for this popup chain
        let root = find_popup_root_surface(&kind).ok().and_then(|root| {
            self.space
                .elements()
                .find(|w| w.toplevel().unwrap().wl_surface() == &root)
                .map(|w| w.toplevel().unwrap().wl_surface().clone())
        });

        let Some(root) = root else { return };

        let ret = self.popups.grab_popup(root, kind, &seat, serial);

        if let Ok(mut grab) = ret {
            if let Some(keyboard) = seat.get_keyboard() {
                if keyboard.is_grabbed()
                    && !(keyboard.has_grab(serial)
                        || keyboard.has_grab(grab.previous_serial().unwrap_or(serial)))
                {
                    grab.ungrab(PopupUngrabStrategy::All);
                    return;
                }
                keyboard.set_focus(self, grab.current_grab(), serial);
                keyboard.set_grab(self, PopupKeyboardGrab::new(&grab), serial);
            }
            if let Some(pointer) = seat.get_pointer() {
                if pointer.is_grabbed()
                    && !(pointer.has_grab(serial)
                        || pointer.has_grab(
                            grab.previous_serial().unwrap_or_else(|| grab.serial()),
                        ))
                {
                    grab.ungrab(PopupUngrabStrategy::All);
                    return;
                }
                pointer.set_grab(self, PopupPointerGrab::new(&grab), serial, Focus::Keep);
            }
        }
    }
}

delegate_xdg_shell!(Compositor);

fn check_grab(
    seat: &Seat<Compositor>,
    surface: &WlSurface,
    serial: Serial,
) -> Option<PointerGrabStartData<Compositor>> {
    let pointer = seat.get_pointer()?;
    if !pointer.has_grab(serial) {
        return None;
    }
    let start_data = pointer.grab_start_data()?;
    let (focus, _) = start_data.focus.as_ref()?;
    if !focus.id().same_client_as(&surface.id()) {
        return None;
    }
    Some(start_data)
}

/// Should be called on `WlSurface::commit`
pub fn handle_commit(popups: &mut PopupManager, space: &Space<Window>, surface: &WlSurface, taskbar_dirty: &mut bool) {
    if let Some(window) = space
        .elements()
        .find(|w| w.toplevel().unwrap().wl_surface() == surface)
        .cloned()
    {
        let (initial_configure_sent, title_changed) = with_states(surface, |states| {
            let data = states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .unwrap()
                .lock()
                .unwrap();
            // Title or app_id changes trigger taskbar update
            let changed = data.title.is_some() || data.app_id.is_some();
            (data.initial_configure_sent, changed)
        });
        if !initial_configure_sent {
            window.toplevel().unwrap().send_configure();
        }
        // Mark dirty on every commit that has title/app_id — the main loop
        // will deduplicate by comparing the serialized JSON.
        if title_changed {
            *taskbar_dirty = true;
        }
    }

    popups.commit(surface);
    if let Some(popup) = popups.find_popup(surface) {
        match popup {
            PopupKind::Xdg(ref xdg) => {
                if !xdg.is_initial_configure_sent() {
                    xdg.send_configure().expect("initial configure failed");
                }
            }
            PopupKind::InputMethod(ref _input_method) => {}
        }
    }
}

impl Compositor {
    fn unconstrain_popup(&self, popup: &PopupSurface) {
        let Ok(root) = find_popup_root_surface(&PopupKind::Xdg(popup.clone())) else {
            return;
        };
        let Some(window) = self
            .space
            .elements()
            .find(|w| w.toplevel().unwrap().wl_surface() == &root)
        else {
            return;
        };

        let output = self.space.outputs().next().unwrap();
        let output_geo = self.space.output_geometry(output).unwrap();
        let window_geo = self.space.element_geometry(window).unwrap();

        let mut target = output_geo;
        target.loc -= get_popup_toplevel_coords(&PopupKind::Xdg(popup.clone()));
        target.loc -= window_geo.loc;

        popup.with_pending_state(|state| {
            state.geometry = state.positioner.get_unconstrained_geometry(target);
        });
    }
}
