use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, HighlightSpacing, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::{anotacoes::Status, app::{App, COLUNAS}, library::Artigo};

const SIMBOLO_SELECAO: &str = "▶ ";

pub fn render(f: &mut Frame, app: &mut App) {
    // Painel de detalhes só quando há altura para a tabela continuar útil
    let altura_detalhes = if f.area().height >= 22 { f.area().height * 2 / 5 } else { 0 };
    let [area_tabela, area_detalhes, area_status] = Layout::vertical([
        Constraint::Min(5),
        Constraint::Length(altura_detalhes),
        Constraint::Length(1),
    ])
    .areas(f.area());
    app.area_tabela = area_tabela;

    render_tabela(f, app, area_tabela);
    if altura_detalhes > 0 {
        let nota = app.state.selected().and_then(|s| app.visiveis.get(s)).filter(|i| app.com_nota.contains(i))
            .zip(app.dir_notas.as_ref()).map(|(&i, dir)| crate::notas::caminho(dir, &app.artigos[i]));
        render_detalhes(f, app.selecionado(), nota, area_detalhes);
    }
    let status = if let Some(tags) = &app.editando_tags {
        Paragraph::new(format!(" Tags (separadas por vírgula): {tags}█   (Enter salva, Esc cancela)")).fg(Color::Black).bg(Color::Cyan)
    } else if let Some(destino) = &app.confirmar_sobrescrita {
        Paragraph::new(format!(" {destino} já existe. Sobrescrever? (s/N)")).fg(Color::White).bg(Color::Red)
    } else if let Some(destino) = &app.destino_bib {
        let n = app.alvos_exportacao(app.exportar_tudo).len();
        Paragraph::new(format!(" Exportar {n} artigos para: {destino}█   (Enter grava, Esc cancela)")).fg(Color::Black).bg(Color::Green)
    } else if app.editando_filtro {
        Paragraph::new(format!(" Filtrar: {}█   (Enter confirma, Esc limpa)", app.filtro)).fg(Color::Black).bg(Color::Yellow)
    } else {
        Paragraph::new(format!(" {}", app.status_msg)).fg(Color::Gray).bg(Color::DarkGray)
    };
    f.render_widget(status, area_status);
}

fn render_tabela(f: &mut Frame, app: &mut App, area: Rect) {
    // Tags só cabem em terminais largos; continuam no painel de detalhes e no filtro
    let n_colunas = if area.width >= 110 { COLUNAS.len() } else { COLUNAS.len() - 1 };
    let larguras = &[
        Constraint::Fill(5), // Título
        Constraint::Fill(2), // Autores
        Constraint::Fill(3), // Venue
        Constraint::Length(6), // Ano
        Constraint::Length(6), // Págs
        Constraint::Length(6), // Refs
        Constraint::Length(10), // Status
        Constraint::Fill(2), // Tags
    ][..n_colunas];
    // Mesmas colunas que a Table vai calcular, para cortar o texto com reticências
    let interno = Rect { width: area.width.saturating_sub(2 + SIMBOLO_SELECAO.chars().count() as u16), ..area };
    let colunas: Vec<usize> = Layout::horizontal(larguras.iter().copied()).spacing(1).split(interno).iter().map(|r| r.width as usize).collect();

    let seta = if app.decrescente { " ▼" } else { " ▲" };
    let header = Row::new(COLUNAS[..n_colunas].iter().enumerate().map(|(i, c)| if i == app.ordem { format!("{c}{seta}") } else { c.to_string() }))
        .style(Style::default().fg(Color::Yellow).bold());

    let rows = app.visiveis.iter().map(|&i| {
        let a = &app.artigos[i];
        let m = &a.meta;
        let marca = if app.marcados.contains(&i) { "● " } else { "" };
        let nota = if app.com_nota.contains(&i) { "✎ " } else { "" };
        let celulas = [
            format!("{marca}{nota}{}", a.titulo),
            autores_curto(m.autores.as_deref()),
            m.venue.clone().unwrap_or_else(|| "—".into()),
            m.ano.clone().unwrap_or_else(|| "—".into()),
            if m.paginas > 0 { m.paginas.to_string() } else { "—".into() },
            m.n_referencias.map_or("—".into(), |n| n.to_string()),
            a.anotacao.status.nome().to_string(),
            if a.anotacao.tags.is_empty() { "—".into() } else { a.anotacao.tags.join(", ") },
        ];
        let cor_status = match a.anotacao.status {
            Status::ParaLer => Color::Reset,
            Status::Lendo => Color::Yellow,
            Status::Lido => Color::Green,
        };
        Row::new(celulas.iter().zip(&colunas).enumerate().map(|(c, (t, &w))| {
            let cell = Cell::from(cortar(t, w));
            if c == 6 { cell.fg(cor_status) } else { cell }
        }))
    });

    let table = Table::new(rows, larguras.iter().copied())
        .header(header)
        .column_spacing(1)
        .block(Block::default().borders(Borders::ALL).title(titulo_tabela(app)))
        .row_highlight_style(Style::default().bg(Color::Blue).fg(Color::White).add_modifier(Modifier::BOLD))
        .highlight_symbol(SIMBOLO_SELECAO)
        .highlight_spacing(HighlightSpacing::Always);

    f.render_stateful_widget(table, area, &mut app.state);
}

fn titulo_tabela(app: &App) -> String {
    let mut t = if app.filtro.is_empty() {
        format!(" Biblioteca de Artigos ({} PDFs) ", app.artigos.len())
    } else {
        format!(" Biblioteca de Artigos ({} de {} PDFs) · filtro: \"{}\" ", app.visiveis.len(), app.artigos.len(), app.filtro)
    };
    if !app.marcados.is_empty() {
        t.push_str(&format!("· {} marcados ", app.marcados.len()));
    }
    t
}

fn render_detalhes(f: &mut Frame, artigo: Option<&Artigo>, nota: Option<std::path::PathBuf>, area: Rect) {
    let bloco = Block::default().borders(Borders::ALL).title(" Detalhes ");
    let Some(a) = artigo else {
        f.render_widget(Paragraph::new("Nenhum artigo selecionado").block(bloco), area);
        return;
    };
    let m = &a.meta;
    let rotulo = |r: &'static str| Span::styled(r, Style::default().fg(Color::Yellow));
    let campo = |r: &'static str, v: Option<String>| v.map(|v| Line::from(vec![rotulo(r), Span::raw(v)]));

    let publicacao = [
        m.venue.clone(),
        m.volume.as_ref().map(|v| format!("vol. {v}")),
        m.intervalo_paginas.as_ref().map(|p| format!("pp. {p}")),
        m.ano.clone(),
    ];
    let identificadores = [
        m.doi.as_ref().map(|d| format!("DOI {d}")),
        m.arxiv.as_ref().map(|x| format!("arXiv {x}")),
        (m.paginas > 0).then(|| format!("{} páginas", m.paginas)),
        m.n_referencias.map(|n| format!("{n} referências")),
        Some(format!("{} ({} KB)", a.nome_arquivo, a.tamanho_kb)),
    ];

    let mut linhas = vec![Line::from(a.titulo.clone()).bold()];
    linhas.extend([
        Some(Line::from(vec![
            rotulo("Status: "),
            Span::raw(a.anotacao.status.nome()),
            rotulo("   Tags: "),
            Span::raw(if a.anotacao.tags.is_empty() { "—".to_string() } else { a.anotacao.tags.join(", ") }),
            rotulo("   Nota: "),
            Span::raw(nota.map_or("— (n cria)".to_string(), |n| format!("✎ {}", n.display()))),
        ])),
        campo("Autores: ", m.autores.clone()),
        campo("Publicação: ", juntar(&publicacao)),
        juntar(&identificadores).map(|t| Line::from(t).fg(Color::Gray)),
        campo("Palavras-chave: ", m.palavras_chave.clone()),
    ].into_iter().flatten());
    if let Some(r) = &m.resumo {
        linhas.push(Line::default());
        linhas.push(Line::from(r.clone()).italic());
    }

    f.render_widget(Paragraph::new(linhas).wrap(Wrap { trim: true }).block(bloco), area);
}

fn juntar(partes: &[Option<String>]) -> Option<String> {
    let v: Vec<&str> = partes.iter().flatten().map(String::as_str).collect();
    (!v.is_empty()).then(|| v.join(" · "))
}

/// "Coupé et al.", "Silva & Doe", "Silva" — a lista completa fica no painel de detalhes
fn autores_curto(autores: Option<&str>) -> String {
    let Some(a) = autores else { return "—".into() };
    let sobrenomes: Vec<&str> = a.split(';').filter_map(|n| n.split_whitespace().last()).collect();
    match sobrenomes.as_slice() {
        [] => "—".into(),
        [s] => s.to_string(),
        [s1, s2] => format!("{s1} & {s2}"),
        [s, ..] => format!("{s} et al."),
    }
}

/// Corta em `largura` colunas terminando com "…"
fn cortar(s: &str, largura: usize) -> String {
    if s.chars().count() <= largura {
        return s.to_string();
    }
    let mut out: String = s.chars().take(largura.saturating_sub(1)).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{config::Config, metadata::PdfMetadata};
    use ratatui::{backend::TestBackend, Terminal};
    use std::path::PathBuf;

    fn tela(app: &mut App, largura: u16, altura: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(largura, altura)).unwrap();
        terminal.draw(|f| render(f, app)).unwrap();
        terminal.backend().buffer().content().iter().map(|c| c.symbol()).collect()
    }

    fn app_exemplo() -> App {
        let artigos = vec![Artigo {
            titulo: "Hippocampal-amygdalo-ventricular atrophy score: Alzheimer disease detection using normative and pathological lifespan models".into(),
            nome_arquivo: "coupe".into(),
            meta: PdfMetadata {
                autores: Some("Pierrick Coupé; José V. Manjon; Boris Mansencal".into()),
                venue: Some("Human Brain Mapping".into()),
                ano: Some("2022".into()),
                paginas: 13,
                n_referencias: Some(35),
                doi: Some("10.1002/hbm.25850".into()),
                ..Default::default()
            },
            anotacao: Default::default(),
            caminho: PathBuf::from("/docs/coupe.pdf"),
            tamanho_kb: 512,
        }];
        App::new(artigos, Config::default(), None)
    }

    #[test]
    fn tabela_corta_textos_longos_e_detalhes_mostram_completo() {
        let conteudo = tela(&mut app_exemplo(), 100, 30);
        assert!(conteudo.contains("Biblioteca de Artigos (1 PDFs)"));
        assert!(conteudo.contains("Coupé et al."));
        assert!(conteudo.contains('…'));
        assert!(conteudo.contains("Pierrick Coupé; José V. Manjon; Boris Mansencal"));
        assert!(conteudo.contains("DOI 10.1002/hbm.25850"));
        assert!(conteudo.contains("35 referências"));
    }

    #[test]
    fn mostra_filtro_e_coluna_ordenada() {
        let mut app = app_exemplo();
        app.inverter_ordem();
        app.editando_filtro = true;
        app.digitar_filtro('x');
        let conteudo = tela(&mut app, 100, 30);
        assert!(conteudo.contains("Título ▼"));
        assert!(conteudo.contains("(0 de 1 PDFs)"));
        assert!(conteudo.contains("Filtrar: x"));
    }

    #[test]
    fn terminal_baixo_oculta_painel_de_detalhes() {
        let conteudo = tela(&mut app_exemplo(), 80, 12);
        assert!(conteudo.contains("Coupé"));
        assert!(conteudo.contains("Para ler"));
        assert!(!conteudo.contains("Tags"), "coluna Tags some em terminal estreito");
        assert!(!conteudo.contains("Detalhes"));
    }

    #[test]
    fn deve_abreviar_autores_e_cortar_com_reticencias() {
        assert_eq!(autores_curto(Some("Ana Silva")), "Silva");
        assert_eq!(autores_curto(Some("Ana Silva; John Doe")), "Silva & Doe");
        assert_eq!(autores_curto(Some("Ana Silva; John Doe; Mary Roe")), "Silva et al.");
        assert_eq!(autores_curto(None), "—");
        assert_eq!(cortar("abcdef", 4), "abc…");
        assert_eq!(cortar("abc", 4), "abc");
    }
}
