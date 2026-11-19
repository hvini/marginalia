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
    app::{App, Lado},
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
    app.carregar_anotacoes(dir_path.join(marginalia::anotacoes::NOME_ARQUIVO));
    app.definir_dir_notas(dir_path.join(marginalia::notas::PASTA));
    let res = run_app(&mut terminal, &mut app, &opener, &dir_path);

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

/// Suspende a TUI, roda o editor no terminal e restaura a TUI ao voltar
fn editar_nota<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    config: &Config,
    nota: &std::path::Path,
) -> io::Result<String> {
    let (programa, args) = config.comando_editor();
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    let resultado = std::process::Command::new(&programa).args(&args).arg(nota).status();
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    terminal.clear()?;
    Ok(match resultado {
        Ok(s) if s.success() => format!("Nota salva: {}", nota.display()),
        Ok(s) => format!("{programa} terminou com {s}; nota em {}", nota.display()),
        Err(e) => format!("Não foi possível abrir o editor '{programa}': {e} (ajuste 'editor' no config.toml)"),
    })
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    opener: &dyn PdfOpener,
    dir: &std::path::Path,
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
            // Grafo de citações: teclas próprias, as da tabela não valem aqui
            Event::Key(key) if key.kind == KeyEventKind::Press && app.vista_grafo.is_some() => match key.code {
                KeyCode::Up | KeyCode::Char('k') => app.grafo_mover(-1),
                KeyCode::Down | KeyCode::Char('j') => app.grafo_mover(1),
                KeyCode::Left | KeyCode::Char('h') => app.grafo_coluna(Lado::Cita),
                KeyCode::Right | KeyCode::Char('l') => app.grafo_coluna(Lado::CitadoPor),
                KeyCode::Enter => app.grafo_centralizar(),
                KeyCode::Backspace => app.grafo_voltar(),
                KeyCode::Char('o') => {
                    if let Some(i) = app.artigo_grafo() {
                        let _ = execute!(io::stdout(), DisableMouseCapture);
                        let _ = io::stdout().flush();
                        app.abrir_pdf(i, opener);
                    }
                }
                KeyCode::Esc | KeyCode::Char('g' | 'q') => app.fechar_grafo(),
                _ => {}
            },
            // Pergunta "sobrescrever o .bib?": só s/y confirma, qualquer outra tecla cancela
            Event::Key(key) if key.kind == KeyEventKind::Press && app.confirmar_sobrescrita.is_some() => match key.code {
                KeyCode::Char('s' | 'S' | 'y' | 'Y') => app.sobrescrever_exportacao(),
                _ => {
                    app.confirmar_sobrescrita = None;
                    app.status_msg = "Exportação cancelada".into();
                }
            },
            // Digitação das tags do selecionado
            Event::Key(key) if key.kind == KeyEventKind::Press && app.editando_tags.is_some() => match key.code {
                KeyCode::Enter => app.confirmar_tags(),
                KeyCode::Esc => app.editando_tags = None,
                KeyCode::Backspace => {
                    app.editando_tags.as_mut().map(String::pop);
                }
                KeyCode::Char(c) => app.editando_tags.as_mut().into_iter().for_each(|t| t.push(c)),
                _ => {}
            },
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
                    KeyCode::Char('g') => app.abrir_grafo(),
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char('/') => app.editando_filtro = true,
                    KeyCode::Char('s') => app.proxima_ordem(),
                    KeyCode::Char('S') => app.inverter_ordem(),
                    KeyCode::Char(' ') => app.alternar_marca(),
                    KeyCode::Char('e') => app.iniciar_exportacao(false),
                    KeyCode::Char('E') => app.iniciar_exportacao(true),
                    KeyCode::Char('l') => app.alternar_status(),
                    KeyCode::Char('t') => app.iniciar_edicao_tags(),
                    KeyCode::Char('n') => {
                        if let Some(nota) = app.preparar_nota() {
                            app.status_msg = editar_nota(terminal, &app.config, &nota)?;
                            // o usuário pode ter apagado o arquivo no editor
                            if let Some(dir) = app.dir_notas.clone() {
                                app.definir_dir_notas(dir);
                            }
                        }
                    }
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
                        // PDFs novos são extraídos; os já conhecidos vêm do cache
                        app.status_msg = match carregar_artigos(dir, Cache::caminho_padrao().as_deref()) {
                            Err(e) => format!("Erro ao reler a biblioteca: {e}"),
                            Ok(artigos) => {
                                let n = artigos.len();
                                app.recarregar_artigos(artigos);
                                aviso.unwrap_or_else(|| format!("Recarregado: {n} artigos | Viewer: {}", app.config.nome_viewer_exibicao()))
                            }
                        };
                    }
                    _ => {}
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
                // no grafo a área da tabela não está na tela: clique selecionaria uma linha invisível
                MouseEventKind::Up(MouseButton::Left) if app.vista_grafo.is_none() => {
                    let _ = execute!(io::stdout(), DisableMouseCapture);
                    let _ = io::stdout().flush();
                    app.lidar_clique_mouse(mouse.column, mouse.row, opener);
                }
                MouseEventKind::ScrollDown if app.vista_grafo.is_some() => app.grafo_mover(1),
                MouseEventKind::ScrollUp if app.vista_grafo.is_some() => app.grafo_mover(-1),
                MouseEventKind::ScrollDown => app.proximo(),
                MouseEventKind::ScrollUp => app.anterior(),
                _ => {}
            },
            _ => {}
        }
    }
}