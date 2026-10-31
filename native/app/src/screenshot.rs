//! Headless screenshots: `espectral-client --screenshot <route> <out.png> [opts]`.
//! FOUNDATION-OWNED.
//!
//! Installs a custom Slint platform (MinimalSoftwareWindow + a hand-pumped
//! event loop), so no display server or GPU is needed. The app boots exactly
//! like the real one (engine, shell, page modules), navigates to `<route>`,
//! pumps events / timers for `--wait` ms so engine calls land, renders one
//! frame with the software renderer and writes it as PNG.
//!
//! Options: --theme dark|light|system  --lang es|en  --accent <id>
//!          --size 1280x800  --wait 1500  --scroll <px>
//!          --open palette|wizard|account|pip|gate   (repeatable)
//!          --query <text>   (palette query, with --open palette)
//!          --click X,Y      (logical px; press + release, then pump 400 ms; repeatable)
//!          --type <text>    (text into the focused input; repeatable)
//!          --key <name>     (Escape|Return|Tab|Space|Down|Up; repeatable)
//!          --nav <route>    (navigate again mid-run; repeatable)
//!          Actions run in command-line order after the first half of --wait.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Platform, WindowAdapter};
use slint::{EventLoopError, PhysicalSize, PlatformError, Rgb8Pixel};

#[derive(Debug, Clone)]
pub struct ShotArgs {
    pub route: String,
    pub out: std::path::PathBuf,
    pub theme: Option<String>,
    pub lang: Option<String>,
    pub accent: Option<String>,
    pub width: u32,
    pub height: u32,
    pub wait_ms: u64,
    pub scroll: f32,
    pub open: Vec<String>,
    pub query: Option<String>,
    pub actions: Vec<Action>,
}

/// A scripted input event (`--click`, `--type`, `--key`, `--nav`).
#[derive(Debug, Clone)]
pub enum Action {
    Click(f32, f32),
    Type(String),
    Key(String),
    Nav(String),
}

impl ShotArgs {
    /// Parse `--screenshot <route> <out.png> [opts]` (args after the program name).
    pub fn parse(args: &[String]) -> Option<ShotArgs> {
        let i = args.iter().position(|a| a == "--screenshot")?;
        let route = args.get(i + 1)?.clone();
        let out = std::path::PathBuf::from(args.get(i + 2)?);
        let mut s = ShotArgs {
            route,
            out,
            theme: None,
            lang: None,
            accent: None,
            width: 1280,
            height: 800,
            wait_ms: 1500,
            scroll: 0.0,
            open: vec![],
            query: None,
            actions: vec![],
        };
        let mut k = 0;
        while k < args.len() {
            let v = args.get(k + 1).cloned();
            match args[k].as_str() {
                "--theme" => s.theme = v,
                "--lang" => s.lang = v,
                "--accent" => s.accent = v,
                "--wait" => s.wait_ms = v.and_then(|x| x.parse().ok()).unwrap_or(s.wait_ms),
                "--scroll" => s.scroll = v.and_then(|x| x.parse().ok()).unwrap_or(0.0),
                "--open" => s.open.extend(v),
                "--query" => s.query = v,
                "--click" => {
                    if let Some((x, y)) = v.as_deref().and_then(|x| x.split_once(',')) {
                        if let (Ok(x), Ok(y)) = (x.trim().parse(), y.trim().parse()) {
                            s.actions.push(Action::Click(x, y));
                        }
                    }
                }
                "--type" => s.actions.extend(v.map(Action::Type)),
                "--key" => s.actions.extend(v.map(Action::Key)),
                "--nav" => s.actions.extend(v.map(Action::Nav)),
                "--size" => {
                    if let Some((w, h)) = v.as_deref().and_then(|x| x.split_once('x')) {
                        s.width = w.parse().unwrap_or(1280);
                        s.height = h.parse().unwrap_or(800);
                    }
                }
                _ => {}
            }
            k += 1;
        }
        Some(s)
    }
}

type Job = Box<dyn FnOnce() + Send>;

struct Queue {
    jobs: Mutex<VecDeque<Job>>,
}

struct Proxy(Arc<Queue>);

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), EventLoopError> {
        Ok(())
    }
    fn invoke_from_event_loop(&self, event: Box<dyn FnOnce() + Send>) -> Result<(), EventLoopError> {
        self.0.jobs.lock().unwrap_or_else(|p| p.into_inner()).push_back(event);
        Ok(())
    }
}

struct HeadlessPlatform {
    window: Rc<MinimalSoftwareWindow>,
    queue: Arc<Queue>,
    start: Instant,
}

impl Platform for HeadlessPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.window.clone())
    }
    fn duration_since_start(&self) -> Duration {
        self.start.elapsed()
    }
    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy(self.queue.clone())))
    }
}

thread_local! {
    static STATE: RefCell<Option<(Rc<MinimalSoftwareWindow>, Arc<Queue>)>> = const { RefCell::new(None) };
}

/// Install the headless platform. Must run before any Slint window is created.
pub fn install(width: u32, height: u32) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    window.set_size(PhysicalSize::new(width, height));
    let queue = Arc::new(Queue { jobs: Mutex::new(VecDeque::new()) });
    STATE.with(|s| *s.borrow_mut() = Some((window.clone(), queue.clone())));
    slint::platform::set_platform(Box::new(HeadlessPlatform { window, queue, start: Instant::now() }))
        .expect("a Slint platform was already initialized");
}

/// Run queued invoke_from_event_loop jobs + timers/animations for `dur`.
///
/// Frames are drawn into a scratch buffer whenever the window is dirty, like
/// the real event loop does: lazily created subtrees (page `if` blocks), their
/// `changed` handlers, Timers, SkinView preloads and `--scroll` only come alive
/// once the item tree has been rendered at least once.
pub fn pump(dur: Duration) {
    let deadline = Instant::now() + dur;
    let (window, queue) = STATE.with(|s| s.borrow().clone()).expect("headless platform not installed");
    let size = window.window().size();
    let mut scratch = vec![Rgb8Pixel::default(); (size.width * size.height) as usize];
    let mut last_draw = Instant::now() - Duration::from_secs(1);
    loop {
        loop {
            let job = queue.jobs.lock().unwrap_or_else(|p| p.into_inner()).pop_front();
            match job {
                Some(j) => j(),
                None => break,
            }
        }
        slint::platform::update_timers_and_animations();
        if last_draw.elapsed() >= Duration::from_millis(40) {
            window.draw_if_needed(|renderer| {
                renderer.render(&mut scratch, size.width as usize);
            });
            last_draw = Instant::now();
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(Duration::from_millis(8));
    }
}

/// Dispatch one scripted input action to the headless window.
pub fn dispatch(action: &Action) {
    use slint::platform::{Key, PointerEventButton, WindowEvent};
    use slint::LogicalPosition;
    let window = STATE.with(|s| s.borrow().as_ref().map(|(w, _)| w.clone())).expect("headless platform not installed");
    let w = window.window();
    match action {
        Action::Click(x, y) => {
            let position = LogicalPosition::new(*x, *y);
            w.dispatch_event(WindowEvent::PointerMoved { position });
            pump(Duration::from_millis(60));
            w.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
            w.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
        }
        Action::Type(text) => {
            for ch in text.chars() {
                let t: slint::SharedString = ch.to_string().into();
                w.dispatch_event(WindowEvent::KeyPressed { text: t.clone() });
                w.dispatch_event(WindowEvent::KeyReleased { text: t });
            }
        }
        Action::Key(name) => {
            let key: char = match name.as_str() {
                "Escape" | "Esc" => Key::Escape.into(),
                "Return" | "Enter" => Key::Return.into(),
                "Tab" => Key::Tab.into(),
                "Space" => ' ',
                "Down" => Key::DownArrow.into(),
                "Up" => Key::UpArrow.into(),
                "Backspace" => Key::Backspace.into(),
                other => other.chars().next().unwrap_or(' '),
            };
            let t: slint::SharedString = key.to_string().into();
            w.dispatch_event(WindowEvent::KeyPressed { text: t.clone() });
            w.dispatch_event(WindowEvent::KeyReleased { text: t });
        }
        Action::Nav(_) => {}
    }
    pump(Duration::from_millis(400));
}

/// Render the current frame and save it as PNG.
pub fn save_png(path: &std::path::Path) -> std::io::Result<()> {
    let window = STATE.with(|s| s.borrow().as_ref().map(|(w, _)| w.clone())).expect("headless platform not installed");
    let size = window.window().size();
    let (w, h) = (size.width, size.height);
    let mut buf = vec![Rgb8Pixel::default(); (w * h) as usize];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut buf, w as usize);
    });
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(std::io::Error::other)?;
    let bytes: Vec<u8> = buf.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
    writer.write_image_data(&bytes).map_err(std::io::Error::other)?;
    Ok(())
}
