use std::{
    env,
    fs,
    io::{self, stdout},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
        MouseEventKind, DisableFocusChange, EnableFocusChange,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    widgets::{Block, Borders, Cell, HighlightSpacing, Paragraph, Row, Table, TableState},
    Terminal,
};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    #[serde(default = "default_viewer")]
    viewer: Option<String>,
    #[serde(default)]
    viewer_args: Vec<String>,
}

fn default_viewer() -> Option<String> {
    Some(String::from("zathura"))
}

impl Default for Config {
    fn default() -> Self {
        Self {
            viewer: default_viewer(),
            viewer_args: Vec::new(),
        }
    }
}

impl Config {
    /// Retorna o caminho ~/.config/marginalia/config.toml
    fn caminho_config() -> Option<PathBuf> {
        dirs::config_dir().map(|dir| dir.join("marginalia").join("config.toml"))
    }

    /// Carrega o dotfile ou cria um arquivo padrão se não existir
    fn carregar_ou_criar() -> (Self, Option<String>) {
        let Some(path) = Self::caminho_config() else {
            return (Self::default(), Some("Diretório de config não encontrado, usando padrão.".into()));
        };

        if !path.exists() {
            let cfg = Self::default();
            if let Some(parent) = path.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let conteudo_inicial = format!(
                "# Ordem de preferência dos visualizadores de PDF\n\
                 # Se nenhum estiver instalado, usa o padrão do sistema.\n{}",
                toml::to_string_pretty(&cfg).unwrap_or_default()
            );
            let _ = fs::write(&path, conteudo_inicial);
            return (cfg, Some(format!("Config criada em: {}", path.display())));
        }

        match fs::read_to_string(&path) {
            Ok(conteudo) => match toml::from_str::<Config>(&conteudo) {
                Ok(cfg) => (cfg, None),
                Err(e) => (
                    Self::default(),
                    Some(format!("Erro no config.toml ({}), usando padrão.", e)),
                ),
            },
            Err(e) => (
                Self::default(),
                Some(format!("Erro ao ler config.toml: {}", e)),
            ),
        }
    }

    fn nome_viewer_exibicao(&self) -> &str {
        self.viewer
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or("padrão do sistema")
    }
}

/// Estrutura que representa cada artigo/PDF encontrado
struct Artigo {
    nome: String,
    caminho: PathBuf,
    tamanho_kb: u64,
}

/// Estado da nossa aplicação TUI
struct App {
    artigos: Vec<Artigo>,
    state: TableState,
    area_tabela: Rect, // Guardamos a área da tabela para calcular cliques do mouse
    status_msg: String,
    config: Config,
}

impl App {
    fn new(artigos: Vec<Artigo>, config: Config, aviso_config: Option<String>) -> Self {
        let mut state = TableState::default();
        if !artigos.is_empty() {
            state.select(Some(0));
        }

        let status_msg = aviso_config.unwrap_or_else(|| {
            format!(
                "Preferência: {:?} | Enter/Clique: abrir | 'r': recarregar config | 'q': sair",
                config.nome_viewer_exibicao()
            )
        });

        Self {
            artigos,
            state,
            area_tabela: Rect::default(),
            status_msg,
            config,
        }
    }

    fn recarregar_config(&mut self) {
        let (nova_config, aviso) = Config::carregar_ou_criar();
        self.config = nova_config;
        self.status_msg = aviso.unwrap_or_else(|| {
            format!("Config recarregada! Viewer: {}", self.config.nome_viewer_exibicao())
        });
    }

    fn proximo(&mut self) {
        if self.artigos.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) if i < self.artigos.len() - 1 => i + 1,
            _ => 0,
        };
        self.state.select(Some(i));
    }

    fn anterior(&mut self) {
        if self.artigos.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => self.artigos.len() - 1,
        };
        self.state.select(Some(i));
    }

    fn abrir_selecionado(&mut self) {
        if let Some(i) = self.state.selected() {
            if let Some(artigo) = self.artigos.get(i) {
                let _ = execute!(io::stdout(), DisableMouseCapture);
                let _ = io::Write::flush(&mut io::stdout());

                match abrir_com_config(&artigo.caminho, &self.config) {
                    Ok(viewer) => {
                        self.status_msg = format!("Abrindo '{}' com {}", artigo.nome, viewer);
                    }
                    Err(e) => {
                        self.status_msg = format!("Erro ao abrir {}: {}", artigo.nome, e);
                    }
                }
            }
        }
    }

    fn lidar_clique_mouse(&mut self, col: u16, row: u16) {
        let inicio_linhas_y = self.area_tabela.y + 2;
        let fim_linhas_y = self.area_tabela.y + self.area_tabela.height.saturating_sub(1);

        if col >= self.area_tabela.x
            && col < self.area_tabela.x + self.area_tabela.width
            && row >= inicio_linhas_y
            && row < fim_linhas_y
        {
            let linha_clicada = (row - inicio_linhas_y) as usize;
            let indice_real = self.state.offset() + linha_clicada;

            if indice_real < self.artigos.len() {
                self.state.select(Some(indice_real));
                self.abrir_selecionado();
            }
        }
    }
}

/// Tenta abrir com: 1) PDF_VIEWER env -> 2) `viewer` do dotfile -> 3) padrão do SO
fn abrir_com_config(caminho: &Path, config: &Config) -> Result<String, String> {
    let viewer_escolhido = env::var("PDF_VIEWER")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| config.viewer.clone().filter(|v| !v.trim().is_empty()));

    if let Some(viewer) = viewer_escolhido {
        let res = Command::new(&viewer)
            .args(&config.viewer_args)
            .arg(caminho)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();

        if res.is_ok() {
            return Ok(viewer);
        }
    }

    // Fallback para o visualizador padrão do sistema operacional
    open::that_detached(caminho)
        .map(|_| String::from("padrão do sistema (fallback)"))
        .map_err(|e| e.to_string())
}

fn carregar_artigos(dir: &Path) -> io::Result<Vec<Artigo>> {
    let mut artigos = Vec::new();

    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext.eq_ignore_ascii_case("pdf") {
                        let nome = path
                            .file_stem()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_string();

                        let metadata = fs::metadata(&path)?;
                        let tamanho_kb = metadata.len() / 1024;

                        artigos.push(Artigo {
                            nome,
                            caminho: path,
                            tamanho_kb,
                        });
                    }
                }
            }
        }
    }

    artigos.sort_by(|a, b| a.nome.to_lowercase().cmp(&b.nome.to_lowercase()));
    Ok(artigos)
}

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let dir_path = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        PathBuf::from(".")
    };

    let artigos = carregar_artigos(&dir_path)?;
    let (config, aviso_config) = Config::carregar_ou_criar();

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
    let res = run_app(&mut terminal, &mut app);

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
) -> io::Result<()> {
    loop {
        terminal.draw(|f| {
            let chunks =
                Layout::vertical([Constraint::Min(5), Constraint::Length(3)]).split(f.area());
            app.area_tabela = chunks[0];

            let header = Row::new(vec!["Título do Artigo", "Tamanho", "Caminho"])
                .style(Style::default().fg(Color::Yellow).bold())
                .height(1);

            let rows: Vec<Row> = app
                .artigos
                .iter()
                .map(|artigo| {
                    Row::new(vec![
                        Cell::from(artigo.nome.clone()),
                        Cell::from(format!("{} KB", artigo.tamanho_kb)),
                        Cell::from(artigo.caminho.to_string_lossy().to_string()),
                    ])
                    .height(1)
                })
                .collect();

            let widths = [
                Constraint::Percentage(50),
                Constraint::Length(12),
                Constraint::Percentage(38),
            ];

            let table = Table::new(rows, widths)
                .header(header)
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title(format!(" Biblioteca de Artigos ({} PDFs) ", app.artigos.len())),
                )
                .row_highlight_style(
                    Style::default()
                        .bg(Color::Blue)
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▶ ")
                .highlight_spacing(HighlightSpacing::Always);

            f.render_stateful_widget(table, chunks[0], &mut app.state);

            let status = Paragraph::new(app.status_msg.clone())
                .block(Block::default().borders(Borders::ALL).title(" Status "));
            f.render_widget(status, chunks[1]);
        })?;

        match event::read()? {
            Event::FocusLost => {
                let _ = execute!(io::stdout(), DisableMouseCapture);
                let _ = io::Write::flush(&mut io::stdout());
            }
            Event::FocusGained => {
                let _ = execute!(io::stdout(), EnableMouseCapture);
                let _ = io::Write::flush(&mut io::stdout());
            }
            Event::Key(key) if key.kind == KeyEventKind::Press => {
                let _ = execute!(io::stdout(), EnableMouseCapture);
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Down | KeyCode::Char('j') => app.proximo(),
                    KeyCode::Up | KeyCode::Char('k') => app.anterior(),
                    KeyCode::Enter => app.abrir_selecionado(),
                    KeyCode::Char('r') => app.recarregar_config(),
                    _ => {}
                }
            }
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::Up(MouseButton::Left) => {
                    app.lidar_clique_mouse(mouse.column, mouse.row);
                }
                MouseEventKind::ScrollDown => app.proximo(),
                MouseEventKind::ScrollUp => app.anterior(),
                _ => {}
            },
            _ => {}
        }
    }
}