use std::{
    env,
    io::{self, stdout, Write},
    path::PathBuf,
};

use crossterm::{
    event::{
        self, DisableFocusChange, DisableMouseCapture, EnableFocusChange, EnableMouseCapture,
        Event, KeyCode, KeyEventKind, MouseButton, MouseEventKind,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};

use marginalia::{
    app::App,
    cache::Cache,
    config::Config,
    library::carregar_artigos,
    opener::{PdfOpener, SystemOpener},
    ui,
};

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let dir_path = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        PathBuf::from(".")
    };

    let artigos = carregar_artigos(&dir_path, Cache::caminho_padrao().as_deref())?;
    let (config, aviso_config) = Config::carregar_ou_criar();
    let opener = SystemOpener;

    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableFocusChange
    )?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(artigos, config, aviso_config);
    let res = run_app(&mut terminal, &mut app, &opener);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableFocusChange
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        println!("Erro na execução: {:?}", err);
    }

    Ok(())
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    opener: &dyn PdfOpener,
) -> io::Result<()> {
    loop {
        terminal.draw(|f| ui::render(f, app))?;

        match event::read()? {
            Event::FocusLost => {
                let _ = execute!(io::stdout(), DisableMouseCapture);
                let _ = io::stdout().flush();
            }
            Event::FocusGained => {
                let _ = execute!(io::stdout(), EnableMouseCapture);
                let _ = io::stdout().flush();
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let _ = execute!(io::stdout(), EnableMouseCapture);
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Down | KeyCode::Char('j') => app.proximo(),
                    KeyCode::Up | KeyCode::Char('k') => app.anterior(),
                    KeyCode::Enter => {
                        let _ = execute!(io::stdout(), DisableMouseCapture);
                        let _ = io::stdout().flush();
                        app.abrir_selecionado(opener);
                    }
                    KeyCode::Char('r') => {
                        let (nova_config, aviso) = Config::carregar_ou_criar();
                        app.config = nova_config;
                        app.status_msg = aviso.unwrap_or_else(|| {
                            format!(
                                "Config recarregada! Viewer: {}",
                                app.config.nome_viewer_exibicao()
                            )
                        });
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::Up(MouseButton::Left) => {
                    let _ = execute!(io::stdout(), DisableMouseCapture);
                    let _ = io::stdout().flush();
                    app.lidar_clique_mouse(mouse.column, mouse.row, opener);
                }
                MouseEventKind::ScrollDown => app.proximo(),
                MouseEventKind::ScrollUp => app.anterior(),
                _ => {}
            },
            _ => {}
        }
    }
}