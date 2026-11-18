use ratatui::{layout::Rect, widgets::TableState};
use crate::{config::Config, library::Artigo, opener::PdfOpener};

pub struct App {
    pub artigos: Vec<Artigo>,
    pub state: TableState,
    pub area_tabela: Rect,
    pub status_msg: String,
    pub config: Config,
}

impl App {
    pub fn new(artigos: Vec<Artigo>, config: Config, aviso_config: Option<String>) -> Self {
        let mut state = TableState::default();
        if !artigos.is_empty() {
            state.select(Some(0));
        }

        let status_msg = aviso_config.unwrap_or_else(|| {
            format!("Viewer: {} | Enter/Clique: abrir", config.nome_viewer_exibicao())
        });

        Self {
            artigos,
            state,
            area_tabela: Rect::default(),
            status_msg,
            config,
        }
    }

    pub fn proximo(&mut self) {
        if self.artigos.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) if i < self.artigos.len() - 1 => i + 1,
            _ => 0,
        };
        self.state.select(Some(i));
    }

    pub fn anterior(&mut self) {
        if self.artigos.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => self.artigos.len() - 1,
        };
        self.state.select(Some(i));
    }

    pub fn abrir_selecionado(&mut self, opener: &dyn PdfOpener) {
        if let Some(i) = self.state.selected() {
            if let Some(artigo) = self.artigos.get(i) {
                match opener.open(&artigo.caminho, &self.config) {
                    Ok(viewer) => {
                        self.status_msg = format!("Abrindo '{}' com {}", artigo.titulo, viewer);
                    }
                    Err(e) => {
                        self.status_msg = format!("Erro ao abrir {}: {}", artigo.titulo, e);
                    }
                }
            }
        }
    }

    pub fn lidar_clique_mouse(&mut self, col: u16, row: u16, opener: &dyn PdfOpener) {
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
                self.abrir_selecionado(opener);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};

    struct MockOpener {
        ultimo_aberto: RefCell<Option<PathBuf>>,
    }

    impl MockOpener {
        fn new() -> Self {
            Self {
                ultimo_aberto: RefCell::new(None),
            }
        }
    }

    impl PdfOpener for MockOpener {
        fn open(&self, caminho: &Path, _config: &Config) -> Result<String, String> {
            *self.ultimo_aberto.borrow_mut() = Some(caminho.to_path_buf());
            Ok("mock-viewer".into())
        }
    }

    fn artigo_teste(titulo: &str, path: &str) -> Artigo {
        Artigo {
            titulo: titulo.into(),
            nome_arquivo: titulo.into(),
            meta: Default::default(),
            caminho: PathBuf::from(path),
            tamanho_kb: 100,
        }
    }

    fn criar_artigos_exemplo() -> Vec<Artigo> {
        vec![
            artigo_teste("Artigo 1", "/tmp/1.pdf"),
            artigo_teste("Artigo 2", "/tmp/2.pdf"),
            artigo_teste("Artigo 3", "/tmp/3.pdf"),
        ]
    }

    #[test]
    fn navegacao_deve_ser_circular() {
        let mut app = App::new(criar_artigos_exemplo(), Config::default(), None);
        assert_eq!(app.state.selected(), Some(0));

        // Voltar no primeiro vai para o último
        app.anterior();
        assert_eq!(app.state.selected(), Some(2));

        // Avançar no último volta para o primeiro
        app.proximo();
        assert_eq!(app.state.selected(), Some(0));
    }

    #[test]
    fn clique_na_linha_deve_selecionar_e_abrir_pdf_correto() {
        let mut app = App::new(criar_artigos_exemplo(), Config::default(), None);
        // Simula uma tabela ocupando de (0,0) até (80, 10)
        // Linha 0: borda superior | Linha 1: cabeçalho | Linha 2: Artigo 1 | Linha 3: Artigo 2
        app.area_tabela = Rect::new(0, 0, 80, 10);
        let opener = MockOpener::new();

        // Clica na linha y = 3 (deve corresponder ao índice 1 -> Artigo 2)
        app.lidar_clique_mouse(10, 3, &opener);

        assert_eq!(app.state.selected(), Some(1));
        assert_eq!(
            *opener.ultimo_aberto.borrow(),
            Some(PathBuf::from("/tmp/2.pdf"))
        );
        assert!(app.status_msg.contains("Artigo 2"));
    }

    #[test]
    fn clique_no_cabecalho_ou_borda_nao_deve_abrir_nada() {
        let mut app = App::new(criar_artigos_exemplo(), Config::default(), None);
        app.area_tabela = Rect::new(0, 0, 80, 10);
        let opener = MockOpener::new();

        // Clica no cabeçalho (y = 1)
        app.lidar_clique_mouse(10, 1, &opener);

        assert_eq!(*opener.ultimo_aberto.borrow(), None);
    }
}