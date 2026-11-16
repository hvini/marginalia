use std::{
    env,
    fs,
    io::{self, stdout},
    path::{Path, PathBuf},
};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, MouseButton,
        MouseEventKind,
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
}

impl App {
    fn new(artigos: Vec<Artigo>) -> Self {
        let mut state = TableState::default();
        if !artigos.is_empty() {
            state.select(Some(0)); // Seleciona o primeiro item por padrão
        }
        Self {
            artigos,
            state,
            area_tabela: Rect::default(),
            status_msg: String::from("Use ↑/↓ ou clique para selecionar. Enter ou clique para abrir. 'q' para sair."),
        }
    }

    fn proximo(&mut self) {
        if self.artigos.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i >= self.artigos.len() - 1 {
                    0
                } else {
                    i + 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    fn anterior(&mut self) {
        if self.artigos.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) => {
                if i == 0 {
                    self.artigos.len() - 1
                } else {
                    i - 1
                }
            }
            None => 0,
        };
        self.state.select(Some(i));
    }

    /// Abre o PDF selecionado usando o programa padrão do sistema operacional
    fn abrir_selecionado(&mut self) {
        if let Some(i) = self.state.selected() {
            if let Some(artigo) = self.artigos.get(i) {
                match open::that(&artigo.caminho) {
                    Ok(_) => {
                        self.status_msg = format!("Abrindo: {}", artigo.nome);
                    }
                    Err(e) => {
                        self.status_msg = format!("Erro ao abrir {}: {}", artigo.nome, e);
                    }
                }
            }
        }
    }

    /// Calcula qual linha foi clicada com base nas coordenadas (x, y) do mouse
    fn lidar_clique_mouse(&mut self, col: u16, row: u16) {
        // Verifica se o clique ocorreu dentro da área interna da tabela
        // +1 na borda superior e +1 no cabeçalho da tabela = offset de 2 linhas
        let inicio_linhas_y = self.area_tabela.y + 2;
        let fim_linhas_y = self.area_tabela.y + self.area_tabela.height.saturating_sub(1);

        if col >= self.area_tabela.x
            && col < self.area_tabela.x + self.area_tabela.width
            && row >= inicio_linhas_y
            && row < fim_linhas_y
        {
            // Calcula o índice visual somado ao deslocamento (scroll) atual da tabela
            let linha_clicada = (row - inicio_linhas_y) as usize;
            let indice_real = self.state.offset() + linha_clicada;

            if indice_real < self.artigos.len() {
                self.state.select(Some(indice_real));
                self.abrir_selecionado();
            }
        }
    }
}

/// Lê os arquivos PDF de um diretório
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

    // Ordena alfabeticamente
    artigos.sort_by(|a, b| a.nome.to_lowercase().cmp(&b.nome.to_lowercase()));
    Ok(artigos)
}

fn main() -> io::Result<()> {
    // Pega o diretório via argumento CLI ou usa o diretório atual "."
    let args: Vec<String> = env::args().collect();
    let dir_path = if args.len() > 1 {
        PathBuf::from(&args[1])
    } else {
        PathBuf::from(".")
    };

    let artigos = carregar_artigos(&dir_path)?;

    // Configuração do Terminal (Raw Mode + Alternate Screen + Mouse)
    enable_raw_mode()?;
    let mut stdout = stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(artigos);
    let res = run_app(&mut terminal, &mut app);

    // Restaura o terminal antes de encerrar (essencial para não quebrar o shell)
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
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
            // Divide a tela: Tabela principal e barra de status inferior
            let chunks = Layout::vertical([Constraint::Min(5), Constraint::Length(3)]).split(f.area());
            app.area_tabela = chunks[0];

            // Cabeçalho da Tabela
            let header = Row::new(vec!["Título do Artigo", "Tamanho", "Caminho"])
                .style(Style::default().fg(Color::Yellow).bold())
                .height(1);

            // Linhas da Tabela
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

            // Largura das colunas
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

            // Barra de Status
            let status = Paragraph::new(app.status_msg.clone())
                .block(Block::default().borders(Borders::ALL).title(" Status "));
            f.render_widget(status, chunks[1]);
        })?;

        // Captura de Eventos (Teclado e Mouse)
        match event::read()? {
            Event::Key(key) if key.kind == KeyEventKind::Press => match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Down | KeyCode::Char('j') => app.proximo(),
                KeyCode::Up | KeyCode::Char('k') => app.anterior(),
                KeyCode::Enter => app.abrir_selecionado(),
                _ => {}
            },
            Event::Mouse(mouse) => match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
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