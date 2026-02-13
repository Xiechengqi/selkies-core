//! Compositor state - ported from smithay's smallvil example

use std::{collections::HashSet, ffi::OsString, os::fd::OwnedFd, sync::Arc};

use smithay::{
    desktop::{PopupManager, Space, Window, WindowSurfaceType},
    input::{Seat, SeatState},
    reexports::{
        calloop::{generic::Generic, EventLoop, Interest, LoopSignal, Mode, PostAction},
        wayland_server::{
            backend::{ClientData, ClientId, DisconnectReason},
            protocol::wl_surface::WlSurface,
            Display, DisplayHandle,
        },
    },
    utils::{Logical, Point},
    wayland::{
        compositor::{CompositorClientState, CompositorState},
        output::OutputManagerState,
        selection::data_device::DataDeviceState,
        shell::xdg::{XdgShellState, decoration::XdgDecorationState},
        shm::ShmState,
        socket::ListeningSocketSource,
        text_input::TextInputManagerState,
    },
};

#[allow(dead_code)]
pub struct Compositor {
    pub start_time: std::time::Instant,
    pub socket_name: OsString,
    pub display_handle: DisplayHandle,

    pub space: Space<Window>,
    pub loop_signal: LoopSignal,

    // Smithay protocol state
    pub compositor_state: CompositorState,
    pub xdg_shell_state: XdgShellState,
    pub shm_state: ShmState,
    pub output_manager_state: OutputManagerState,
    pub seat_state: SeatState<Compositor>,
    pub data_device_state: DataDeviceState,
    pub xdg_decoration_state: XdgDecorationState,
    pub popups: PopupManager,
    pub text_input_manager_state: TextInputManagerState,

    pub seat: Seat<Self>,

    /// Current cursor status from Wayland clients, updated by SeatHandler::cursor_image
    pub cursor_status: smithay::input::pointer::CursorImageStatus,

    /// Set by surface commit, cleared after rendering
    pub needs_redraw: bool,

    /// Text pending for clipboard paste injection
    pub pending_paste: Option<String>,

    /// Clipboard content set by a Wayland client (to broadcast to browser)
    pub clipboard_outgoing: Option<String>,

    /// Pipe read fd for reading client clipboard data
    pub clipboard_read_fd: Option<OwnedFd>,

    /// Surfaces that have already had their CSD titlebar offset applied
    pub titlebar_adjusted: HashSet<u32>,

    /// Number of CSD compensation attempts (limit retries)
    pub csd_retry_count: u32,

    /// Window list changed — needs broadcast to frontend
    pub taskbar_dirty: bool,

    /// Currently focused surface ID for taskbar highlighting
    pub focused_surface_id: Option<u32>,

    /// Registry of active window surfaces (stable order for taskbar)
    pub window_registry: Vec<WlSurface>,

    /// Surface protocol IDs that were identified as dialogs at creation time
    pub dialog_surfaces: HashSet<u32>,

    /// Surface protocol IDs that had Fullscreen removed (browsers)
    pub browser_unfullscreened: HashSet<u32>,

    /// Whether keyboard focus needs to be re-sent after the first pointer enter.
    /// Chromium's Ozone/Wayland layer may ignore keyboard events received before
    /// wl_pointer.enter, so we re-send wl_keyboard.enter on first pointer motion.
    pub kbd_focus_needs_reenter: bool,
}

impl Compositor {
    pub fn new(event_loop: &mut EventLoop<Self>, display: Display<Self>) -> Self {
        let start_time = std::time::Instant::now();
        let dh = display.handle();

        let compositor_state = CompositorState::new::<Self>(&dh);
        let xdg_shell_state = XdgShellState::new::<Self>(&dh);
        let shm_state = ShmState::new::<Self>(&dh, vec![]);
        let popups = PopupManager::default();
        let output_manager_state = OutputManagerState::new_with_xdg_output::<Self>(&dh);
        let data_device_state = DataDeviceState::new::<Self>(&dh);
        let xdg_decoration_state = XdgDecorationState::new::<Self>(&dh);
        let text_input_manager_state = TextInputManagerState::new::<Self>(&dh);

        let mut seat_state = SeatState::new();
        let mut seat: Seat<Self> = seat_state.new_wl_seat(&dh, "ivnc");
        seat.add_keyboard(Default::default(), 200, 25).unwrap();
        seat.add_pointer();

        let space = Space::default();
        let socket_name = Self::init_wayland_listener(display, event_loop);
        let loop_signal = event_loop.get_signal();

        Self {
            start_time,
            display_handle: dh,
            space,
            loop_signal,
            socket_name,
            compositor_state,
            xdg_shell_state,
            shm_state,
            output_manager_state,
            seat_state,
            data_device_state,
            xdg_decoration_state,
            text_input_manager_state,
            popups,
            seat,
            cursor_status: smithay::input::pointer::CursorImageStatus::default_named(),
            needs_redraw: false,
            pending_paste: None,
            clipboard_outgoing: None,
            clipboard_read_fd: None,
            titlebar_adjusted: HashSet::new(),
            csd_retry_count: 0,
            taskbar_dirty: false,
            focused_surface_id: None,
            window_registry: Vec::new(),
            dialog_surfaces: HashSet::new(),
            browser_unfullscreened: HashSet::new(),
            kbd_focus_needs_reenter: true,
        }
    }

    fn init_wayland_listener(
        display: Display<Compositor>,
        event_loop: &mut EventLoop<Self>,
    ) -> OsString {
        let listening_socket = ListeningSocketSource::new_auto().unwrap();
        let socket_name = listening_socket.socket_name().to_os_string();
        let loop_handle = event_loop.handle();

        loop_handle
            .insert_source(listening_socket, move |client_stream, _, state| {
                state
                    .display_handle
                    .insert_client(client_stream, Arc::new(ClientState::default()))
                    .unwrap();
            })
            .expect("Failed to init the wayland event source.");

        loop_handle
            .insert_source(
                Generic::new(display, Interest::READ, Mode::Level),
                |_, display, state| {
                    unsafe {
                        display.get_mut().dispatch_clients(state).unwrap();
                    }
                    Ok(PostAction::Continue)
                },
            )
            .unwrap();

        socket_name
    }

    pub fn surface_under(
        &self,
        pos: Point<f64, Logical>,
    ) -> Option<(WlSurface, Point<f64, Logical>)> {
        self.space.element_under(pos).and_then(|(window, location)| {
            window
                .surface_under(pos - location.to_f64(), WindowSurfaceType::ALL)
                .map(|(s, p)| (s, (p + location).to_f64()))
        })
    }
}

#[derive(Default)]
pub struct ClientState {
    pub compositor_state: CompositorClientState,
}

impl ClientData for ClientState {
    fn initialized(&self, _client_id: ClientId) {}
    fn disconnected(&self, _client_id: ClientId, _reason: DisconnectReason) {}
}
