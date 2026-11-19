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
    app.destino_padrao = dir_path.join("referencias.bib").to_string_lossy().into_owned();
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
            // Digitação do caminho do .bib
            Event::Key(key) if key.kind == KeyEventKind::Press && app.destino_bib.is_some() => match key.code {
                KeyCode::Enter => app.confirmar_exportacao(),
                KeyCode::Esc => app.destino_bib = None,
                KeyCode::Backspace => {
                    app.destino_bib.as_mut().map(String::pop);
                }
                KeyCode::Char(c) => app.destino_bib.as_mut().into_iter().for_each(|d| d.push(c)),
                _ => {}
            },
            // Modo de digitação do filtro: as teclas viram texto, não comandos
            Event::Key(key) if key.kind == KeyEventKind::Press && app.editando_filtro => match key.code {
                KeyCode::Enter => app.editando_filtro = false,
                KeyCode::Esc => app.limpar_filtro(),
                KeyCode::Backspace => app.apagar_filtro(),
                KeyCode::Down => app.proximo(),
                KeyCode::Up => app.anterior(),
                KeyCode::Char(c) => app.digitar_filtro(c),
                _ => {}
            },
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let _ = execute!(io::stdout(), EnableMouseCapture);
                match key.code {
                    KeyCode::Esc if !app.filtro.is_empty() => app.limpar_filtro(),
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('/') => app.editando_filtro = true,
                    KeyCode::Char('s') => app.proxima_ordem(),
                    KeyCode::Char('S') => app.inverter_ordem(),
                    KeyCode::Char(' ') => app.alternar_marca(),
                    KeyCode::Char('e') => app.iniciar_exportacao(false),
                    KeyCode::Char('E') => app.iniciar_exportacao(true),
                    KeyCode::Char('y') => app.copiar_bibtex(false),
                    KeyCode::Char('Y') => app.copiar_bibtex(true),
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