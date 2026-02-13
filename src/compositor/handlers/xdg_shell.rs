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

impl XdgShellHandler for Compositor {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let has_parent = surface.parent().is_some();

        // Detect dialog: either has explicit parent, or another toplevel from the
        // same client already exists (GTK3 FileChooserDialog doesn't set parent
        // via xdg_toplevel protocol, but opens as a second toplevel from same client)
        let new_surface_id = surface.wl_surface().id();
        let same_client_toplevel_exists = self.space.elements().any(|w| {
            let existing_id = w.toplevel().unwrap().wl_surface().id();
            existing_id.same_client_as(&new_surface_id)
        });
        let is_dialog = has_parent || same_client_toplevel_exists;

        log::info!("new_toplevel: is_dialog={} (parent={}, same_client={})",
            is_dialog, has_parent, same_client_toplevel_exists);

        let window = Window::new_wayland_window(surface.clone());

        // Extract output geometry before mutably borrowing space
        let output_geo = self.space.outputs().next()
            .and_then(|o| self.space.output_geometry(o));

        self.space.map_element(window, (0, 0), false);

        if let Some(output_geo) = output_geo {
            if is_dialog {
                // Dialog: don't fullscreen, force a size that fits with CSD decorations
                surface.with_pending_state(|state| {
                    let w = ((output_geo.size.w * 2) / 3).max(400);
                    let h = ((output_geo.size.h * 2) / 3).max(300);
                    state.size = Some((w, h).into());
                });
                surface.send_pending_configure();
            } else {
                // Normal toplevel: only set size here. Fullscreen state will be
                // added later in commit handler once app_id is known (browsers
                // should NOT get Fullscreen as they hide their address bar).
                surface.with_pending_state(|state| {
                    state.size = Some((output_geo.size.w, output_geo.size.h).into());
                });
                surface.send_pending_configure();
            }
        }

        // Register surface for stable taskbar ordering — skip dialogs
        if !is_dialog {
            self.window_registry.push(surface.wl_surface().clone());
        }

        // Remember which surfaces are dialogs (for commit handler centering/sizing)
        if is_dialog {
            self.dialog_surfaces.insert(surface.wl_surface().id().protocol_id());
        }

        // Auto-focus the new window so it receives keyboard input
        let keyboard = self.seat.get_keyboard().unwrap();
        let focus_serial = smithay::utils::SERIAL_COUNTER.next_serial();
        keyboard.set_focus(self, Some(surface.wl_surface().clone()), focus_serial);

        self.taskbar_dirty = true;
    }

    fn parent_changed(&mut self, surface: ToplevelSurface) {
        // When a toplevel gets a parent (becomes a dialog), undo fullscreen
        // and force a smaller size so the dialog fits on screen with CSD decorations.
        if surface.parent().is_some() {
            let output_geo = self.space.outputs().next()
                .and_then(|o| self.space.output_geometry(o));
            log::info!("parent_changed: dialog detected, removing fullscreen");
            self.dialog_surfaces.insert(surface.wl_surface().id().protocol_id());
            surface.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Fullscreen);
                state.states.unset(xdg_toplevel::State::Maximized);
                if let Some(geo) = output_geo {
                    let w = ((geo.size.w * 2) / 3).max(400);
                    let h = ((geo.size.h * 2) / 3).max(300);
                    state.size = Some((w, h).into());
                }
            });
            surface.send_pending_configure();
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

        // Remove from dialog tracking
        self.dialog_surfaces.remove(&surface.wl_surface().id().protocol_id());

        // Remove only the destroyed surface from window registry (not siblings)
        let surf_id = surface.wl_surface().id();
        self.window_registry.retain(|wl| wl.id() != surf_id);

        // Only kill the owning process if no other toplevels from the same client remain.
        // Otherwise closing a dialog would kill the entire application.
        let has_sibling = self.space.elements().any(|w| {
            let wl_id = w.toplevel().unwrap().wl_surface().id();
            wl_id != surf_id && wl_id.same_client_as(&surf_id)
        });
        if !has_sibling {
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
