//! Tela de abertura: logo e navegador de pastas para escolher a biblioteca sem passar o caminho na linha de comando.

use ratatui::{
    layout::{Alignment, Constraint, Flex, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Frame,
};
use std::{
    fs,
    path::{Path, PathBuf},
};

const LOGO: &str = r"                                _                _  _
 _ __ ___    __ _  _ __   __ _ (_) _ __    __ _ | |(_)  __ _
| '_ ` _ \  / _` || '__| / _` || || '_ \  / _` || || | / _` |
| | | | | || (_| || |   | (_| || || | | || (_| || || || (_| |
|_| |_| |_| \__,_||_|    \__, ||_||_| |_| \__,_||_||_| \__,_|
                         |___/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// Usar a pasta atual como biblioteca
    Abrir,
    Subir,
    Pasta { nome: String, pdfs: usize },
}

pub struct Navegador {
    pub dir: PathBuf,
    pub itens: Vec<Item>,
    pub pdfs_aqui: usize,
    pub state: ListState,
}

impl Navegador {
    pub fn new(dir: &Path) -> Self {
        let mut nav = Navegador { dir: PathBuf::new(), itens: Vec::new(), pdfs_aqui: 0, state: ListState::default() };
        nav.ir_para(dir, None);
        nav
    }

    /// Lista `dir`; `selecionar` é a subpasta a deixar selecionada (a que acabamos de deixar ao subir)
    fn ir_para(&mut self, dir: &Path, selecionar: Option<&str>) {
        self.dir = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        // pastas ocultas e sem permissão de leitura ficam de fora
        let mut pastas: Vec<(String, usize)> = fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with('.'))
            .map(|n| {
                let pdfs = contar_pdfs(&self.dir.join(&n));
                (n, pdfs)
            })
            .collect();
        pastas.sort_by_key(|(n, _)| n.to_lowercase());

        self.pdfs_aqui = contar_pdfs(&self.dir);
        self.itens = vec![Item::Abrir];
        if self.dir.parent().is_some() {
            self.itens.push(Item::Subir);
        }
        self.itens.extend(pastas.into_iter().map(|(nome, pdfs)| Item::Pasta { nome, pdfs }));
        let alvo = selecionar.and_then(|s| self.itens.iter().position(|i| matches!(i, Item::Pasta { nome, .. } if nome == s)));
        self.state.select(Some(alvo.unwrap_or(0)));
    }

    pub fn mover(&mut self, passo: isize) {
        let n = self.itens.len() as isize;
        let atual = self.state.selected().unwrap_or(0) as isize;
        self.state.select(Some((atual + passo).rem_euclid(n) as usize));
    }

    pub fn subir(&mut self) {
        let atual = self.dir.file_name().map(|n| n.to_string_lossy().into_owned());
        if let Some(pai) = self.dir.parent().map(Path::to_path_buf) {
            self.ir_para(&pai, atual.as_deref());
        }
    }

    pub fn ir_para_inicio(&mut self) {
        if let Some(home) = dirs::home_dir() {
            self.ir_para(&home, None);
        }
    }

    /// Enter: entra na subpasta, sobe, ou devolve a pasta escolhida como biblioteca
    pub fn ativar(&mut self) -> Option<PathBuf> {
        match self.itens.get(self.state.selected()?)?.clone() {
            Item::Abrir => return Some(self.dir.clone()),
            Item::Subir => self.subir(),
            Item::Pasta { nome, .. } => {
                let destino = self.dir.join(nome);
                self.ir_para(&destino, None);
            }
        }
        None
    }
}

fn contar_pdfs(dir: &Path) -> usize {
    fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x.eq_ignore_ascii_case("pdf")))
        .count()
}

fn n_pdfs(n: usize) -> String {
    if n == 1 { "1 PDF".into() } else { format!("{n} PDFs") }
}

/// "/home/ana/papers" → "~/papers"
fn abreviar(dir: &Path) -> String {
    match dirs::home_dir().and_then(|h| dir.strip_prefix(&h).ok().map(Path::to_path_buf)) {
        Some(resto) if resto.as_os_str().is_empty() => "~".into(),
        Some(resto) => format!("~/{}", resto.display()),
        None => dir.display().to_string(),
    }
}

/// Logo no alto (se houver altura) e o conteúdo da tela abaixo dele
fn com_logo(f: &mut Frame) -> Rect {
    let area = f.area();
    let altura_logo = LOGO.lines().count() as u16;
    if area.height < altura_logo + 14 {
        return area;
    }
    let largura_logo = LOGO.lines().map(|l| l.chars().count()).max().unwrap_or(0) as u16;
    let [_, logo, subtitulo, _, resto] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(altura_logo),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(0),
    ])
    .areas(area);
    let [logo] = Layout::horizontal([Constraint::Length(largura_logo)]).flex(Flex::Center).areas(logo);
    f.render_widget(Paragraph::new(LOGO).fg(Color::Yellow), logo);
    f.render_widget(Paragraph::new("gerenciador de referências local").fg(Color::DarkGray).alignment(Alignment::Center), subtitulo);
    resto
}

pub fn render(f: &mut Frame, nav: &mut Navegador) {
    let resto = com_logo(f);
    // caixa do tamanho da lista (até o espaço disponível), com a ajuda logo abaixo
    let altura = (nav.itens.len() as u16 + 2).min(resto.height.saturating_sub(1));
    let [lista, ajuda] = Layout::vertical([Constraint::Length(altura), Constraint::Length(1)]).areas(resto);
    let [lista] = Layout::horizontal([Constraint::Length(70.min(lista.width))]).flex(Flex::Center).areas(lista);

    let cinza = Style::default().fg(Color::DarkGray);
    let itens: Vec<ListItem> = nav
        .itens
        .iter()
        .map(|item| {
            let linha = match item {
                Item::Abrir if nav.pdfs_aqui > 0 => Line::from(vec![
                    Span::styled("Abrir esta pasta", Style::default().fg(Color::Green).bold()),
                    Span::styled(format!("  ·  {}", n_pdfs(nav.pdfs_aqui)), Style::default().fg(Color::Green)),
                ]),
                Item::Abrir => Line::from(vec![Span::raw("Abrir esta pasta"), Span::styled("  ·  nenhum PDF aqui", cinza)]),
                Item::Subir => Line::styled("../", cinza),
                Item::Pasta { nome, pdfs: 0 } => Line::from(format!("{nome}/")),
                Item::Pasta { nome, pdfs } => Line::from(vec![
                    Span::raw(format!("{nome}/")),
                    Span::styled(format!("  {}", n_pdfs(*pdfs)), Style::default().fg(Color::Green)),
                ]),
            };
            ListItem::new(linha)
        })
        .collect();
    let bloco = Block::default().borders(Borders::ALL).title(format!(" {} ", abreviar(&nav.dir))).padding(ratatui::widgets::Padding::horizontal(1));
    let lista_widget = List::new(itens)
        .block(bloco)
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED))
        .highlight_symbol("▶ ");
    f.render_stateful_widget(lista_widget, lista, &mut nav.state);

    let tecla = |t: &'static str| Span::styled(t, Style::default().fg(Color::Yellow));
    let texto = Line::from(vec![
        tecla("↑↓"), Span::raw(" escolher  "), tecla("Enter"), Span::raw(" entrar/abrir  "),
        tecla("←"), Span::raw(" pasta acima  "), tecla("~"), Span::raw(" pasta pessoal  "), tecla("q"), Span::raw(" sair"),
    ]);
    f.render_widget(Paragraph::new(texto).alignment(Alignment::Center), ajuda);
}

/// Mostrada enquanto os metadados são extraídos (a primeira abertura de uma pasta pode levar alguns segundos)
pub fn render_carregando(f: &mut Frame, dir: &Path) {
    let resto = com_logo(f);
    let [linha] = Layout::vertical([Constraint::Length(1)]).flex(Flex::Center).areas(resto);
    f.render_widget(Paragraph::new(format!("Lendo os PDFs de {}…", abreviar(dir))).alignment(Alignment::Center), linha);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navega_entre_pastas_e_escolhe_a_atual() {
        let raiz = tempfile::tempdir().unwrap();
        let r = raiz.path();
        for d in ["papers", "Outros", ".oculta"] {
            fs::create_dir(r.join(d)).unwrap();
        }
        fs::write(r.join("papers/a.pdf"), "").unwrap();
        fs::write(r.join("papers/b.PDF"), "").unwrap();
        fs::write(r.join("papers/notas.txt"), "").unwrap();

        let mut nav = Navegador::new(r);
        assert_eq!(nav.pdfs_aqui, 0);
        assert_eq!(nav.itens[2..], [Item::Pasta { nome: "Outros".into(), pdfs: 0 }, Item::Pasta { nome: "papers".into(), pdfs: 2 }]);

        nav.state.select(Some(3));
        assert_eq!(nav.ativar(), None); // entrou em papers
        assert_eq!(nav.pdfs_aqui, 2);
        assert_eq!(nav.state.selected(), Some(0));

        nav.subir(); // volta com papers selecionada
        assert_eq!(nav.itens[nav.state.selected().unwrap()], Item::Pasta { nome: "papers".into(), pdfs: 2 });

        nav.ativar();
        assert_eq!(nav.ativar(), Some(r.canonicalize().unwrap().join("papers")));
        nav.mover(-1);
        assert_eq!(nav.state.selected(), Some(nav.itens.len() - 1), "circular");
    }
}
