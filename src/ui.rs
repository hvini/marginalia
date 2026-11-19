use ratatui::{
    layout::{Constraint, Flex, Layout, Margin, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    layout::Alignment,
    widgets::{Block, BorderType, Borders, Cell, HighlightSpacing, Paragraph, Row, Table, Wrap},
    Frame,
};

use crate::{anotacoes::Status, app::{App, Lado, VistaGrafo, COLUNAS}, library::Artigo};

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

    if let Some(v) = &app.vista_grafo {
        render_grafo(f, app, v, area_tabela);
    } else {
        render_tabela(f, app, area_tabela);
    }
    if altura_detalhes > 0 {
        // no grafo, os detalhes acompanham o cursor do grafo
        let sel = if app.vista_grafo.is_some() { app.artigo_grafo() } else { app.indice_selecionado() };
        let nota = sel.filter(|i| app.com_nota.contains(i))
            .zip(app.dir_notas.as_ref()).map(|(i, dir)| crate::notas::caminho(dir, &app.artigos[i]));
        let citacoes = sel.map(|i| linha_citacoes(app, i)).unwrap_or_default();
        render_detalhes(f, sel.map(|i| &app.artigos[i]), nota, citacoes, area_detalhes);
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

fn render_detalhes(f: &mut Frame, artigo: Option<&Artigo>, nota: Option<std::path::PathBuf>, citacoes: Option<Line<'static>>, area: Rect) {
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
        citacoes,
        juntar(&identificadores).map(|t| Line::from(t).fg(Color::Gray)),
        campo("Palavras-chave: ", m.palavras_chave.clone()),
    ].into_iter().flatten());
    if let Some(r) = &m.resumo {
        linhas.push(Line::default());
        linhas.push(Line::from(r.clone()).italic());
    }

    f.render_widget(Paragraph::new(linhas).wrap(Wrap { trim: true }).block(bloco), area);
}

/// "Cita: Coupé 2022, Silva 2019   Citado por: Doe 2023" — só citações entre PDFs locais
fn linha_citacoes(app: &App, i: usize) -> Option<Line<'static>> {
    let nomes = |v: &[usize]| v.iter().map(|&j| rotulo_curto(&app.artigos[j])).collect::<Vec<_>>().join(", ");
    let (cita, citado) = (&app.grafo.cita[i], &app.grafo.citado_por[i]);
    if cita.is_empty() && citado.is_empty() {
        return None;
    }
    let mut spans = Vec::new();
    if !cita.is_empty() {
        spans.extend([Span::styled("Cita: ", Style::default().fg(COR_CITA)), Span::raw(nomes(cita) + "   ")]);
    }
    if !citado.is_empty() {
        spans.extend([Span::styled("Citado por: ", Style::default().fg(COR_CITADO)), Span::raw(nomes(citado))]);
    }
    Some(Line::from(spans))
}

const COR_CITA: Color = Color::Cyan;
const COR_CITADO: Color = Color::Magenta;

/// "Coupé 2022"; sem autor, o começo do título
fn rotulo_curto(a: &Artigo) -> String {
    let autor = a.meta.autores.as_deref().and_then(|s| s.split(';').next()?.split_whitespace().last().map(String::from));
    let nome = cortar(&autor.unwrap_or_else(|| a.titulo.clone()), 14);
    a.meta.ano.as_ref().map_or(nome.clone(), |ano| format!("{nome} {ano}"))
}

/// Grafo centrado no foco: à esquerda o que ele cita, à direita quem o cita (do mais antigo ao mais recente).
fn render_grafo(f: &mut Frame, app: &App, v: &VistaGrafo, area: Rect) {
    let tecla = |t: &'static str| Span::styled(t, Style::default().fg(Color::Yellow));
    let ajuda = Line::from(vec![
        Span::raw(" "), tecla("↑↓"), Span::raw(" escolher  "), tecla("←→"), Span::raw(" coluna  "),
        tecla("Enter"), Span::raw(" centralizar  "), tecla("Bksp"), Span::raw(" voltar  "),
        tecla("o"), Span::raw(" abrir PDF  "), tecla("Esc"), Span::raw(" sair "),
    ]);
    // trilha de navegação: "Heister 2011 › Bartos 2019 › Coupé 2022"
    let trilha: Vec<String> = v.historico.iter().chain([&v.foco]).map(|&i| rotulo_curto(&app.artigos[i])).collect();
    let bloco = Block::default().borders(Borders::ALL).title(format!(" Citações · {} ", trilha.join(" › "))).title_bottom(ajuda);
    let interno = bloco.inner(area);
    f.render_widget(bloco, area);

    let util = interno.inner(Margin { horizontal: 3, vertical: 1 });
    if util.height < 5 || util.width < 40 {
        return;
    }

    // colunas laterais só com "Sobrenome Ano" (o título completo está nos detalhes, que seguem o cursor),
    // com a largura do maior rótulo; o conjunto fica centralizado
    let colunas = [Lado::Cita, Lado::CitadoPor].map(|lado| app.coluna_grafo(lado));
    let cabecalhos = [format!("CITA ({})", colunas[0].len()), format!("CITADO POR ({})", colunas[1].len())];
    const VAZIA: &str = "nenhum PDF local";
    let largura_col = |k: usize| {
        let maior = colunas[k].iter().map(|&i| rotulo_curto(&app.artigos[i]).chars().count() + 2).max().unwrap_or(0);
        maior.max(cabecalhos[k].len()).max(VAZIA.len()).min(24) as u16
    };
    let (larg_esq, larg_dir) = (largura_col(0), largura_col(1));
    let largura_foco = util.width.saturating_sub(larg_esq + larg_dir + 8 + 6).clamp(24, 40);
    let [esq, con_esq, centro, con_dir, dir] = Layout::horizontal([
        Constraint::Length(larg_esq),
        Constraint::Length(4),
        Constraint::Length(largura_foco),
        Constraint::Length(4),
        Constraint::Length(larg_dir),
    ])
    .flex(Flex::Center)
    .areas(util);
    let corpo_y = util.y + 2;
    let corpo_h = util.height - 2;
    let meio = corpo_h / 2; // linha (relativa ao corpo) onde os conectores encontram o foco

    // cartão do foco
    let foco = &app.artigos[v.foco];
    let publicacao = juntar(&[foco.meta.venue.clone(), foco.meta.ano.clone()]).unwrap_or_default();
    // shortcut: estimativa da quebra de linha (palavras longas podem gastar uma linha a mais)
    let largura_texto = largura_foco.saturating_sub(4).max(1) as usize;
    let linhas = |t: &str| t.chars().count().div_ceil(largura_texto).max(1) as u16;
    let altura_cartao = (linhas(&foco.titulo) + linhas(&publicacao) + 3).min(corpo_h);
    let cartao = Rect { x: centro.x, y: corpo_y + meio.saturating_sub(altura_cartao / 2), width: centro.width, height: altura_cartao };
    let texto_cartao = vec![
        Line::from(foco.titulo.clone()).bold(),
        Line::default(),
        Line::from(publicacao).fg(Color::Gray),
    ];
    f.render_widget(
        Paragraph::new(texto_cartao).wrap(Wrap { trim: true }).alignment(Alignment::Center).block(
            Block::default().borders(Borders::ALL).border_type(BorderType::Thick).border_style(Style::default().fg(Color::Yellow))
                .title(format!(" {} ", rotulo_curto(foco))).title_alignment(Alignment::Center)
                .padding(ratatui::widgets::Padding::horizontal(1)),
        ),
        cartao,
    );
    f.render_widget(Paragraph::new("EM FOCO").fg(Color::DarkGray).alignment(Alignment::Center), Rect { height: 1, ..centro });

    for (k, (lado, coluna, area_conector, cor)) in [
        (Lado::Cita, esq, con_esq, COR_CITA),
        (Lado::CitadoPor, dir, con_dir, COR_CITADO),
    ]
    .into_iter()
    .enumerate()
    {
        let ids = &colunas[k];
        let ativo = v.lado == lado;
        let alinhamento = if lado == Lado::Cita { Alignment::Right } else { Alignment::Left };
        let em = |linha: u16| Rect { y: corpo_y + linha, height: 1, ..coluna };
        let mut estilo_cab = Style::default().fg(cor);
        if ativo {
            estilo_cab = estilo_cab.add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        }
        f.render_widget(Paragraph::new(cabecalhos[k].as_str()).style(estilo_cab).alignment(alinhamento), Rect { height: 1, ..coluna });
        if ids.is_empty() {
            f.render_widget(Paragraph::new(VAZIA).fg(Color::DarkGray).alignment(alinhamento), em(meio));
            continue;
        }

        // poucos artigos: uma linha de respiro entre eles; muitos: um por linha e a janela rola com o cursor,
        // reservando a primeira e a última linha para os indicadores "↑ N" / "↓ N"
        let n = ids.len() as u16;
        let (passo, cabem) = if n * 2 - 1 <= corpo_h {
            (2, n)
        } else if n <= corpo_h {
            (1, n)
        } else {
            (1, corpo_h.saturating_sub(2).max(1))
        };
        let cursor = if ativo { v.linha } else { 0 };
        let inicio = (cursor + 1).saturating_sub(cabem as usize);
        let mostrados = &ids[inicio..ids.len().min(inicio + cabem as usize)];
        let altura_bloco = (mostrados.len() as u16 - 1) * passo + 1;
        let rolando = n > cabem;
        let topo = if rolando { 1 } else { meio.saturating_sub(altura_bloco / 2).min(corpo_h - altura_bloco) };

        let mut ligacoes = Vec::new();
        for (j, &i) in mostrados.iter().enumerate() {
            let linha = topo + j as u16 * passo;
            ligacoes.push(linha);
            let estilo = if ativo && inicio + j == v.linha {
                Style::default().fg(Color::Black).bg(Color::Yellow).bold()
            } else {
                Style::default().fg(cor)
            };
            let rotulo = format!(" {} ", cortar(&rotulo_curto(&app.artigos[i]), coluna.width.saturating_sub(2) as usize));
            f.render_widget(Paragraph::new(Span::styled(rotulo, estilo)).alignment(alinhamento), em(linha));
        }
        if rolando {
            let indicador = |t: String| Paragraph::new(t).fg(Color::DarkGray).alignment(alinhamento);
            if inicio > 0 {
                f.render_widget(indicador(format!("↑ {} ", inicio)), em(0));
            }
            let abaixo = ids.len() - inicio - mostrados.len();
            if abaixo > 0 {
                f.render_widget(indicador(format!("↓ {} ", abaixo)), em(corpo_h - 1));
            }
        }
        let desenho: Vec<Line> = conector(corpo_h, &ligacoes, meio, lado == Lado::Cita).into_iter().map(Line::from).collect();
        f.render_widget(Paragraph::new(desenho).fg(cor), Rect { y: corpo_y, height: corpo_h, ..area_conector });
    }
}

/// Coluna de 4 caracteres ligando as linhas `ligacoes` (dos artigos) à linha `meio` (o foco) por um barramento
/// vertical. `artigos_a_esquerda`: os artigos estão à esquerda do conector e o foco à direita.
fn conector(altura: u16, ligacoes: &[u16], meio: u16, artigos_a_esquerda: bool) -> Vec<String> {
    let (min, max) = ligacoes.iter().fold((meio, meio), |(a, b), &l| (a.min(l), b.max(l)));
    (0..altura)
        .map(|r| {
            let artigo = ligacoes.contains(&r);
            let foco = r == meio;
            let (esq, dir) = if artigos_a_esquerda { (artigo, foco) } else { (foco, artigo) };
            let barra = if r < min || r > max {
                ' '
            } else {
                match (esq, dir, r > min, r < max) {
                    (true, true, true, true) => '┼',
                    (true, false, true, true) => '┤',
                    (false, true, true, true) => '├',
                    (false, false, true, true) => '│',
                    (true, true, false, true) => '┬',
                    (true, true, true, false) => '┴',
                    (true, false, false, true) => '┐',
                    (true, false, true, false) => '┘',
                    (false, true, false, true) => '┌',
                    (false, true, true, false) => '└',
                    _ => '─', // segmento único, sem barramento
                }
            };
            let tracos = |b: bool| if b { "──" } else { "  " };
            let fim = |b: bool| if b { "─" } else { " " };
            if artigos_a_esquerda {
                format!("{}{barra}{}", tracos(esq), fim(dir))
            } else {
                format!("{}{barra}{}", fim(esq), tracos(dir))
            }
        })
        .collect()
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
    fn conector_liga_artigos_ao_foco_pelo_barramento() {
        // artigos nas linhas 0 e 4, foco na 2, artigos à esquerda
        assert_eq!(conector(5, &[0, 4], 2, true), ["──┐ ", "  │ ", "  ├─", "  │ ", "──┘ "]);
        // espelhado, artigo único na linha do foco: traço reto
        assert_eq!(conector(3, &[1], 1, false), ["    ", "────", "    "]);
        // artigo acima do foco, à direita
        assert_eq!(conector(3, &[0], 2, false), [" ┌──", " │  ", "─┘  "]);
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
