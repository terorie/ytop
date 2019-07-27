#![feature(async_await)]

mod args;
mod colorscheme;
mod utils;
mod widgets;

use std::fs;
use std::io;
use std::path::Path;
use std::thread;
use std::time::Duration;

use crossbeam_channel::{select, tick, unbounded, Receiver};
use crossterm::{AlternateScreen, InputEvent, KeyEvent, MouseEvent};
use futures::future::join_all;
use futures::join;
use platform_dirs::{AppDirs, AppUI};
use structopt::StructOpt;
use tui::backend::{Backend, CrosstermBackend};
use tui::layout::{Constraint, Direction, Layout};
use tui::widgets::Widget;
use tui::Terminal;

use args::Args;
use widgets::*;

struct Widgets {
    battery_widget: Option<BatteryWidget>,
    cpu_widget: CpuWidget,
    disk_widget: Option<DiskWidget>,
    help_menu: HelpMenu,
    mem_widget: MemWidget,
    net_widget: Option<NetWidget>,
    proc_widget: ProcWidget,
    statusbar: Option<Statusbar>,
    temp_widget: Option<TempWidget>,
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend>> {
    let screen = AlternateScreen::to_alternate(true)?;
    let backend = CrosstermBackend::with_alternate_screen(screen)?;
    let mut terminal = Terminal::new(backend)?;
    terminal.hide_cursor()?;
    terminal.clear()?;
    Ok(terminal)
}

fn setup_ui_events() -> Receiver<InputEvent> {
    let (ui_events_sender, ui_events_receiver) = unbounded();
    thread::spawn(move || {
        let _screen = crossterm::RawScreen::into_raw_mode().unwrap(); // TODO: unwrap
        let input = crossterm::input();
        input.enable_mouse_mode().unwrap(); // TODO: unwrap
        let mut reader = input.read_sync();
        loop {
            ui_events_sender.send(reader.next().unwrap()).unwrap(); // TODO: unwraps
        }
    });
    ui_events_receiver
}

fn setup_logfile(logfile_path: &Path) {
    fs::create_dir_all(logfile_path.parent().unwrap()).unwrap(); // TODO: unwrap
    let logfile = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(logfile_path)
        .unwrap(); // TODO: unwrap
    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}]: {}",
                chrono::Local::now().format("[%Y-%m-%d][%H:%M:%S]"),
                record.target(),
                record.level(),
                message
            ))
        })
        .chain(logfile)
        .apply()
        .unwrap(); // TODO: unwrap
}

fn read_colorscheme(
    config_folder: &Path,
    colorscheme: &args::Colorscheme,
) -> serde_json::Result<colorscheme::Colorscheme> {
    match colorscheme {
        args::Colorscheme::Custom(name) => serde_json::from_str(
            &fs::read_to_string(config_folder.join(name).with_extension("json")).unwrap(), // TODO: unwrap
        ),
        _ => {
            let json_string = match colorscheme {
                args::Colorscheme::Default => include_str!("../colorschemes/default.json"),
                args::Colorscheme::DefaultDark => include_str!("../colorschemes/default-dark.json"),
                args::Colorscheme::SolarizedDark => {
                    include_str!("../colorschemes/solarized-dark.json")
                }
                args::Colorscheme::Monokai => include_str!("../colorschemes/monokai.json"),
                args::Colorscheme::Vice => include_str!("../colorschemes/vice.json"),
                _ => unreachable!(),
            };
            Ok(serde_json::from_str(json_string).unwrap())
        }
    }
}

fn setup_widgets(args: &Args, colorscheme: &colorscheme::Colorscheme) -> Widgets {
    let battery_widget = Some(BatteryWidget::new());
    let cpu_widget = CpuWidget::new(Duration::from_secs(1), args.average_cpu, args.per_cpu);
    let disk_widget = Some(DiskWidget::new());
    let help_menu = HelpMenu::new();
    let mem_widget = MemWidget::new(Duration::from_secs(1));
    let net_widget = Some(NetWidget::new());
    let proc_widget = ProcWidget::new();
    let statusbar = Some(Statusbar::new());
    let temp_widget = Some(TempWidget::new());

    Widgets {
        battery_widget,
        cpu_widget,
        disk_widget,
        help_menu,
        mem_widget,
        net_widget,
        proc_widget,
        statusbar,
        temp_widget,
    }
}

async fn update_widgets(widgets: &mut Widgets, ticks: i64) {
    let cpu = widgets.cpu_widget.update();
    let mem = widgets.mem_widget.update();
    let proc = widgets.proc_widget.update();
    if let (Some(disk_widget), Some(net_widget), Some(temp_widget)) = (
        widgets.disk_widget.as_mut(),
        widgets.net_widget.as_mut(),
        widgets.temp_widget.as_mut(),
    ) {
        let disk = disk_widget.update();
        let net = net_widget.update();
        let temp = temp_widget.update();
        if let Some(battery_widget) = widgets.battery_widget.as_mut() {
            let battery = battery_widget.update();
            join!(cpu, mem, proc, disk, net, temp, battery);
        } else {
            join!(cpu, mem, proc, disk, net, temp);
        }
    } else {
        join!(cpu, mem, proc);
    }
}

fn draw_widgets<B: Backend>(terminal: &mut Terminal<B>, widgets: &mut Widgets) -> io::Result<()> {
    terminal.draw(|mut frame| {
        let vertical_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints(
                [
                    Constraint::Ratio(1, 3),
                    Constraint::Ratio(1, 3),
                    Constraint::Ratio(1, 3),
                ]
                .as_ref(),
            )
            .split(frame.size());
        widgets.cpu_widget.render(&mut frame, vertical_chunks[0]);
        let middle_horizontal_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 3), Constraint::Ratio(2, 3)].as_ref())
            .split(vertical_chunks[1]);
        widgets
            .mem_widget
            .render(&mut frame, middle_horizontal_chunks[1]);
        let middle_left_vertical_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)].as_ref())
            .split(middle_horizontal_chunks[0]);
        widgets
            .disk_widget
            .as_mut()
            .unwrap() // TODO: unwrap
            .render(&mut frame, middle_left_vertical_chunks[0]);
        widgets
            .temp_widget
            .as_mut()
            .unwrap() // TODO: unwrap
            .render(&mut frame, middle_left_vertical_chunks[1]);
        let bottom_horizontal_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Ratio(1, 2), Constraint::Ratio(1, 2)].as_ref())
            .split(vertical_chunks[2]);
        widgets
            .net_widget
            .as_mut()
            .unwrap() // TODO: unwrap
            .render(&mut frame, bottom_horizontal_chunks[0]);
        widgets
            .proc_widget
            .render(&mut frame, bottom_horizontal_chunks[1]);
    })
}

fn draw_help_menu<B: Backend>(
    terminal: &mut Terminal<B>,
    help_menu: &mut HelpMenu,
) -> io::Result<()> {
    terminal.draw(|mut frame| {})
}

#[tokio::main]
async fn main() {
    let args = Args::from_args();
    let mut show_help_menu = false;

    let program_name = env!("CARGO_PKG_NAME");
    let app_dirs = AppDirs::new(Some(program_name), AppUI::CommandLine).unwrap(); // TODO: unwrap
    let logfile_path = app_dirs.state_dir.join("errors.log");

    let colorscheme = read_colorscheme(&app_dirs.config_dir, &args.colorscheme).unwrap(); // TODO: unwrap
    let mut widgets = setup_widgets(&args, &colorscheme);

    setup_logfile(&logfile_path);
    let mut terminal = setup_terminal().unwrap(); // TODO: unwrap

    let mut ticks = 0;
    let ticker = tick(Duration::from_secs(1));
    let ui_events_receiver = setup_ui_events();

    update_widgets(&mut widgets, ticks).await;
    draw_widgets(&mut terminal, &mut widgets).unwrap(); // TODO: unwrap

    loop {
        select! {
            recv(ticker) -> _ => {
                ticks = (ticks + 1) % 60;
                update_widgets(&mut widgets, ticks).await;
                if !show_help_menu {
                    draw_widgets(&mut terminal, &mut widgets).unwrap(); // TODO: unwrap
                }
            }
            recv(ui_events_receiver) -> message => {
                match message.unwrap() { // TODO: unwrap
                    InputEvent::Keyboard(key_event) => {
                        match key_event {
                            KeyEvent::Char(c) => match c {
                                'q' => break,
                                '?' => {
                                    show_help_menu = !show_help_menu;
                                    if show_help_menu {
                                        draw_help_menu(&mut terminal, &mut widgets.help_menu).unwrap(); // TODO: unwrap
                                    } else {
                                        draw_widgets(&mut terminal, &mut widgets).unwrap(); // TODO: unwrap
                                    }
                                },
                                _ => {}
                            },
                            KeyEvent::Ctrl(c) => match c {
                                'c' => break,
                                _ => {},
                            },
                            KeyEvent::Esc => {
                                if show_help_menu {
                                    show_help_menu = false;
                                    draw_widgets(&mut terminal, &mut widgets).unwrap(); // TODO: unwrap
                                }
                            }
                            _ => {}
                        }
                    }
                    InputEvent::Mouse(mouse_event) => match mouse_event {
                        _ => {}
                    }
                    _ => {}
                }
            }
        }
    }
}
