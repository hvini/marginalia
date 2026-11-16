use ratatui::{
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style, Stylize},
    widgets::{Block, Borders, Cell, HighlightSpacing, Paragraph, Row, Table},
    Frame,
};

use crate::app::App;

pub fn render(f: &mut Frame, app: &mut App) {
    let chunks = Layout::vertical([Constraint::Min(5), Constraint::Length(3)]).split(f.area());
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, library::Artigo};
    use ratatui::{backend::TestBackend, Terminal};
    use std::path::PathBuf;

    #[test]
    fn deve_renderizar_tabela_e_status_corretamente() {
        let artigos = vec![Artigo {
            nome: "Arquitetura Limpa".into(),
            caminho: PathBuf::from("/docs/clean.pdf"),
            tamanho_kb: 512,
        }];
        let mut app = App::new(artigos, Config::default(), None);

        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|f| render(f, &mut app)).unwrap();

        let buffer = terminal.backend().buffer();
        let conteudo_tela: String = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect();

        assert!(conteudo_tela.contains("Biblioteca de Artigos (1 PDFs)"));
        assert!(conteudo_tela.contains("Arquitetura Limpa"));
        assert!(conteudo_tela.contains("512 KB"));
        assert!(conteudo_tela.contains("Status"));
    }
}