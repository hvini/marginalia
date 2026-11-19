use std::{cmp::Ordering, collections::BTreeSet, path::{Path, PathBuf}};

use ratatui::{layout::Rect, widgets::TableState};
use crate::{anotacoes::{self, Anotacao, Anotacoes}, config::Config, library::Artigo, opener::PdfOpener};

/// Colunas da tabela, na ordem de exibição; `App::ordem` indexa este array.
pub const COLUNAS: [&str; 8] = ["Título", "Autores", "Conferência / Journal", "Ano", "Págs", "Refs", "Status", "Tags"];

pub struct App {
    pub artigos: Vec<Artigo>,
    /// Índices em `artigos` que passam no filtro, já ordenados. A seleção da tabela indexa esta lista.
    pub visiveis: Vec<usize>,
    pub state: TableState,
    pub area_tabela: Rect,
    pub status_msg: String,
    pub config: Config,
    pub filtro: String,
    pub editando_filtro: bool,
    pub ordem: usize,
    pub decrescente: bool,
    /// Índices em `artigos` marcados com Espaço para exportação
    pub marcados: BTreeSet<usize>,
    /// `Some(caminho)` enquanto o usuário edita o destino do .bib
    pub destino_bib: Option<String>,
    pub exportar_tudo: bool,
    pub destino_padrao: String,
    /// Mantido vivo enquanto o app roda: no X11 o conteúdo copiado pertence a este processo
    area_transferencia: Option<arboard::Clipboard>,
    /// Todas as anotações do arquivo, inclusive de PDFs ausentes agora (não podem ser perdidas ao salvar)
    pub anotacoes: Anotacoes,
    /// `None` se não há onde salvar ou se o arquivo existente estava ilegível
    pub arquivo_anotacoes: Option<PathBuf>,
    /// `Some(texto)` enquanto o usuário edita as tags do selecionado
    pub editando_tags: Option<String>,
    /// Pasta das notas Markdown; `None` desativa notas
    pub dir_notas: Option<PathBuf>,
    /// Índices em `artigos` que têm nota
    pub com_nota: BTreeSet<usize>,
}

impl App {
    pub fn new(artigos: Vec<Artigo>, config: Config, aviso_config: Option<String>) -> Self {
        let status_msg = aviso_config.unwrap_or_else(|| {
            format!(
                "Viewer: {} | Enter: abrir | /: filtrar | s/S: ordenar | Espaço: marcar | e/E: exportar .bib | y/Y: copiar | l: status | t: tags | n: nota",
                config.nome_viewer_exibicao()
            )
        });

        let mut app = Self {
            artigos,
            visiveis: Vec::new(),
            state: TableState::default(),
            area_tabela: Rect::default(),
            status_msg,
            config,
            filtro: String::new(),
            editando_filtro: false,
            ordem: 0,
            decrescente: false,
            marcados: BTreeSet::new(),
            destino_bib: None,
            exportar_tudo: false,
            destino_padrao: "referencias.bib".into(),
            area_transferencia: None,
            anotacoes: Anotacoes::new(),
            arquivo_anotacoes: None,
            editando_tags: None,
            dir_notas: None,
            com_nota: BTreeSet::new(),
        };
        app.atualizar_visiveis();
        app
    }

    pub fn carregar_anotacoes(&mut self, arquivo: PathBuf) {
        match anotacoes::carregar(&arquivo) {
            Ok(mapa) => {
                for a in &mut self.artigos {
                    a.anotacao = mapa.get(&anotacoes::chave(&a.caminho)).cloned().unwrap_or_default();
                }
                self.anotacoes = mapa;
                self.arquivo_anotacoes = Some(arquivo);
            }
            Err(e) => {
                self.status_msg = format!("{} ilegível ({e}); status e tags não serão salvos", arquivo.display());
            }
        }
        self.atualizar_visiveis();
    }

    /// Altera a anotação do selecionado e persiste imediatamente
    fn anotar(&mut self, f: impl FnOnce(&mut Anotacao)) {
        let Some(&i) = self.state.selected().and_then(|s| self.visiveis.get(s)) else { return };
        let artigo = &mut self.artigos[i];
        f(&mut artigo.anotacao);
        self.anotacoes.insert(anotacoes::chave(&artigo.caminho), artigo.anotacao.clone());
        if let Some(arquivo) = &self.arquivo_anotacoes {
            if let Err(e) = anotacoes::salvar(arquivo, &self.anotacoes) {
                self.status_msg = format!("Erro ao salvar anotações: {e}");
            }
        }
        self.atualizar_visiveis();
    }

    pub fn alternar_status(&mut self) {
        self.anotar(|a| a.status = a.status.proximo());
    }

    pub fn iniciar_edicao_tags(&mut self) {
        if let Some(a) = self.selecionado() {
            self.editando_tags = Some(a.anotacao.tags.join(", "));
        }
    }

    pub fn confirmar_tags(&mut self) {
        if let Some(texto) = self.editando_tags.take() {
            self.anotar(|a| a.tags = anotacoes::parse_tags(&texto));
        }
    }

    /// Define a pasta das notas e verifica quais artigos já têm nota (refazer após editar)
    pub fn definir_dir_notas(&mut self, dir: PathBuf) {
        self.com_nota = (0..self.artigos.len()).filter(|&i| crate::notas::caminho(&dir, &self.artigos[i]).exists()).collect();
        self.dir_notas = Some(dir);
    }

    /// Cria (se preciso) a nota do selecionado e devolve o caminho para abrir no editor
    pub fn preparar_nota(&mut self) -> Option<PathBuf> {
        let i = *self.state.selected().and_then(|s| self.visiveis.get(s))?;
        let dir = self.dir_notas.as_ref()?;
        match crate::notas::garantir(dir, &self.artigos[i]) {
            Ok(nota) => {
                self.com_nota.insert(i);
                Some(nota)
            }
            Err(e) => {
                self.status_msg = format!("Erro ao criar nota: {e}");
                None
            }
        }
    }

    pub fn selecionado(&self) -> Option<&Artigo> {
        self.state.selected().and_then(|i| self.visiveis.get(i)).map(|&i| &self.artigos[i])
    }

    /// Reaplica filtro e ordenação, mantendo selecionado o mesmo artigo se ele continuar visível.
    pub fn atualizar_visiveis(&mut self) {
        let anterior = self.state.selected().and_then(|i| self.visiveis.get(i)).copied();
        let termos: Vec<String> = self.filtro.to_lowercase().split_whitespace().map(String::from).collect();

        self.visiveis = (0..self.artigos.len())
            .filter(|&i| termos.iter().all(|t| passa(&self.artigos[i], t)))
            .collect();
        let (col, desc) = (self.ordem, self.decrescente);
        let artigos = &self.artigos;
        self.visiveis.sort_by(|&a, &b| comparar(&artigos[a], &artigos[b], col, desc));

        let nova = anterior.and_then(|a| self.visiveis.iter().position(|&i| i == a));
        self.state.select(nova.or((!self.visiveis.is_empty()).then_some(0)));
    }

    /// Marca/desmarca o artigo selecionado e desce para o próximo
    pub fn alternar_marca(&mut self) {
        let Some(&i) = self.state.selected().and_then(|s| self.visiveis.get(s)) else { return };
        if !self.marcados.remove(&i) {
            self.marcados.insert(i);
        }
        self.proximo();
    }

    /// `tudo`: biblioteca inteira; senão os marcados, ou, sem marcados, a lista visível (com filtro, se houver)
    pub fn alvos_exportacao(&self, tudo: bool) -> Vec<&Artigo> {
        if tudo {
            self.artigos.iter().collect()
        } else if !self.marcados.is_empty() {
            self.marcados.iter().map(|&i| &self.artigos[i]).collect()
        } else {
            self.visiveis.iter().map(|&i| &self.artigos[i]).collect()
        }
    }

    pub fn iniciar_exportacao(&mut self, tudo: bool) {
        self.exportar_tudo = tudo;
        self.destino_bib = Some(self.destino_padrao.clone());
    }

    pub fn confirmar_exportacao(&mut self) {
        let Some(destino) = self.destino_bib.take() else { return };
        let alvos = self.alvos_exportacao(self.exportar_tudo);
        self.status_msg = if alvos.is_empty() {
            "Nada para exportar".into()
        } else {
            match crate::bibtex::exportar(&alvos, Path::new(&destino)) {
                Ok(()) => format!("{} referências exportadas para {destino}", alvos.len()),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => format!("{destino} já existe; escolha outro nome (e)"),
                Err(e) => format!("Erro ao exportar para {destino}: {e}"),
            }
        };
    }

    /// Copia o BibTeX dos mesmos alvos da exportação (`tudo` como em `E`)
    pub fn copiar_bibtex(&mut self, tudo: bool) {
        let alvos = self.alvos_exportacao(tudo);
        if alvos.is_empty() {
            self.status_msg = "Nada para copiar".into();
            return;
        }
        let (n, texto) = (alvos.len(), crate::bibtex::gerar(&alvos));
        if self.area_transferencia.is_none() {
            match arboard::Clipboard::new() {
                Ok(c) => self.area_transferencia = Some(c),
                Err(e) => {
                    self.status_msg = format!("Área de transferência indisponível: {e}");
                    return;
                }
            }
        }
        self.status_msg = match self.area_transferencia.as_mut().map(|c| c.set_text(texto)) {
            Some(Ok(())) => format!("{n} referências copiadas para a área de transferência"),
            Some(Err(e)) => format!("Erro ao copiar: {e}"),
            None => unreachable!("inicializada acima"),
        };
    }

    pub fn proxima_ordem(&mut self) {
        self.ordem = (self.ordem + 1) % COLUNAS.len();
        self.atualizar_visiveis();
    }

    pub fn inverter_ordem(&mut self) {
        self.decrescente = !self.decrescente;
        self.atualizar_visiveis();
    }

    pub fn digitar_filtro(&mut self, c: char) {
        self.filtro.push(c);
        self.atualizar_visiveis();
    }

    pub fn apagar_filtro(&mut self) {
        self.filtro.pop();
        self.atualizar_visiveis();
    }

    pub fn limpar_filtro(&mut self) {
        self.filtro.clear();
        self.editando_filtro = false;
        self.atualizar_visiveis();
    }

    pub fn proximo(&mut self) {
        if self.visiveis.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) if i < self.visiveis.len() - 1 => i + 1,
            _ => 0,
        };
        self.state.select(Some(i));
    }

    pub fn anterior(&mut self) {
        if self.visiveis.is_empty() {
            return;
        }
        let i = match self.state.selected() {
            Some(i) if i > 0 => i - 1,
            _ => self.visiveis.len() - 1,
        };
        self.state.select(Some(i));
    }

    pub fn abrir_selecionado(&mut self, opener: &dyn PdfOpener) {
        if let Some(artigo) = self.selecionado() {
            self.status_msg = match opener.open(&artigo.caminho, &self.config) {
                Ok(viewer) => format!("Abrindo '{}' com {}", artigo.titulo, viewer),
                Err(e) => format!("Erro ao abrir {}: {}", artigo.titulo, e),
            };
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

            if indice_real < self.visiveis.len() {
                self.state.select(Some(indice_real));
                self.abrir_selecionado(opener);
            }
        }
    }
}

/// Termo do filtro (já em minúsculas): `tag:x` e `status:x` comparam por prefixo só naquele campo
/// (prefixo para a lista reagir enquanto se digita); os demais procuram em todos os campos.
fn passa(a: &Artigo, termo: &str) -> bool {
    if let Some(v) = termo.strip_prefix("tag:") {
        return a.anotacao.tags.iter().any(|t| t.to_lowercase().starts_with(v));
    }
    if let Some(v) = termo.strip_prefix("status:") {
        let so_letras = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
        return so_letras(a.anotacao.status.nome()).starts_with(&so_letras(v));
    }
    texto_busca(a).contains(termo)
}

/// Tudo em que o filtro procura, em minúsculas
fn texto_busca(a: &Artigo) -> String {
    let m = &a.meta;
    let tags = a.anotacao.tags.join(" ");
    let status = a.anotacao.status.nome().to_string();
    [Some(&a.titulo), m.autores.as_ref(), m.venue.as_ref(), m.ano.as_ref(), m.palavras_chave.as_ref(), m.doi.as_ref(), Some(&a.nome_arquivo), Some(&tags), Some(&status)]
        .into_iter()
        .flatten()
        .map(|s| s.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum Chave {
    Num(u64),
    Texto(String),
}

fn chave(a: &Artigo, coluna: usize) -> Option<Chave> {
    let m = &a.meta;
    let texto = |s: Option<&String>| s.map(|s| Chave::Texto(s.to_lowercase()));
    match coluna {
        0 => texto(Some(&a.titulo)),
        // sobrenome do primeiro autor, como aparece na coluna
        1 => m.autores.as_deref().and_then(|s| s.split(';').next()?.split_whitespace().last()).map(|s| Chave::Texto(s.to_lowercase())),
        2 => texto(m.venue.as_ref()),
        3 => m.ano.as_deref().and_then(|a| a.parse().ok()).map(Chave::Num),
        4 => (m.paginas > 0).then_some(Chave::Num(m.paginas as u64)),
        5 => m.n_referencias.map(|n| Chave::Num(n as u64)),
        6 => Some(Chave::Num(a.anotacao.status as u64)),
        _ => texto(a.anotacao.tags.first()),
    }
}

/// Valores ausentes ficam sempre no fim, em qualquer direção; empate desempata pelo título.
fn comparar(a: &Artigo, b: &Artigo, coluna: usize, decrescente: bool) -> Ordering {
    let o = match (chave(a, coluna), chave(b, coluna)) {
        (None, None) => Ordering::Equal,
        (None, _) => Ordering::Greater,
        (_, None) => Ordering::Less,
        (Some(x), Some(y)) if decrescente => y.cmp(&x),
        (Some(x), Some(y)) => x.cmp(&y),
    };
    o.then_with(|| a.titulo.to_lowercase().cmp(&b.titulo.to_lowercase()))
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
        artigo_com(titulo, path, None, None)
    }

    fn artigo_com(titulo: &str, path: &str, ano: Option<&str>, autores: Option<&str>) -> Artigo {
        Artigo {
            titulo: titulo.into(),
            nome_arquivo: titulo.into(),
            anotacao: Default::default(),
            meta: crate::metadata::PdfMetadata {
                ano: ano.map(String::from),
                autores: autores.map(String::from),
                ..Default::default()
            },
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

    fn titulos_visiveis(app: &App) -> Vec<&str> {
        app.visiveis.iter().map(|&i| app.artigos[i].titulo.as_str()).collect()
    }

    #[test]
    fn filtro_combina_termos_em_qualquer_campo_e_ignora_caixa() {
        let artigos = vec![
            artigo_com("Deep Learning", "/a.pdf", Some("2020"), Some("Ana Silva")),
            artigo_com("Graph Search", "/b.pdf", Some("2019"), Some("John Doe")),
            artigo_com("Deep Graphs", "/c.pdf", Some("2021"), Some("Mary Silva")),
        ];
        let mut app = App::new(artigos, Config::default(), None);
        for c in "SILVA deep".chars() {
            app.digitar_filtro(c);
        }
        assert_eq!(titulos_visiveis(&app), ["Deep Graphs", "Deep Learning"]);
        app.limpar_filtro();
        assert_eq!(app.visiveis.len(), 3);
    }

    #[test]
    fn filtro_sem_resultados_limpa_selecao_e_nao_abre_nada() {
        let mut app = App::new(criar_artigos_exemplo(), Config::default(), None);
        app.digitar_filtro('z');
        assert_eq!(app.state.selected(), None);
        let opener = MockOpener::new();
        app.abrir_selecionado(&opener);
        app.proximo();
        assert_eq!(*opener.ultimo_aberto.borrow(), None);
    }

    #[test]
    fn ordenacao_por_ano_deixa_ausentes_no_fim_e_mantem_selecao() {
        let artigos = vec![
            artigo_com("A", "/a.pdf", Some("2020"), None),
            artigo_com("B", "/b.pdf", None, None),
            artigo_com("C", "/c.pdf", Some("2018"), None),
        ];
        let mut app = App::new(artigos, Config::default(), None);
        app.proximo(); // seleciona "B"
        app.ordem = 3; // Ano
        app.atualizar_visiveis();
        assert_eq!(titulos_visiveis(&app), ["C", "A", "B"]);
        assert_eq!(app.selecionado().unwrap().titulo, "B");
        app.inverter_ordem();
        assert_eq!(titulos_visiveis(&app), ["A", "C", "B"]);
    }

    #[test]
    fn clique_usa_a_lista_filtrada() {
        let mut app = App::new(criar_artigos_exemplo(), Config::default(), None);
        app.digitar_filtro('3');
        app.area_tabela = Rect::new(0, 0, 80, 10);
        let opener = MockOpener::new();
        app.lidar_clique_mouse(10, 2, &opener);
        assert_eq!(*opener.ultimo_aberto.borrow(), Some(PathBuf::from("/tmp/3.pdf")));
    }

    #[test]
    fn exporta_marcados_senao_visiveis_e_tudo_ignora_filtro() {
        let mut app = App::new(criar_artigos_exemplo(), Config::default(), None);
        app.digitar_filtro('2');
        assert_eq!(app.alvos_exportacao(false).len(), 1);
        app.limpar_filtro(); // seleção continua em "Artigo 2"
        app.anterior();
        app.alternar_marca(); // marca "Artigo 1" e desce
        app.alternar_marca(); // marca "Artigo 2"
        let marcados: Vec<&str> = app.alvos_exportacao(false).iter().map(|a| a.titulo.as_str()).collect();
        assert_eq!(marcados, ["Artigo 1", "Artigo 2"]);
        app.digitar_filtro('1');
        assert_eq!(app.alvos_exportacao(true).len(), 3);
    }

    #[test]
    fn exportacao_grava_arquivo_e_informa_no_status() {
        let dir = tempfile::tempdir().unwrap();
        let destino = dir.path().join("x.bib");
        let mut app = App::new(criar_artigos_exemplo(), Config::default(), None);
        app.destino_padrao = destino.to_string_lossy().into_owned();
        app.iniciar_exportacao(false);
        app.confirmar_exportacao();
        assert!(app.status_msg.starts_with("3 referências exportadas"));
        assert_eq!(std::fs::read_to_string(&destino).unwrap().matches("@misc{").count(), 3);
        app.iniciar_exportacao(false);
        app.confirmar_exportacao();
        assert!(app.status_msg.contains("já existe"));
    }

    #[test]
    fn copiar_sem_alvos_nao_acessa_area_de_transferencia() {
        let mut app = App::new(criar_artigos_exemplo(), Config::default(), None);
        app.digitar_filtro('z');
        app.copiar_bibtex(false);
        assert_eq!(app.status_msg, "Nada para copiar");
        assert!(app.area_transferencia.is_none());
    }

    #[test]
    fn status_e_tags_persistem_e_filtram() {
        let dir = tempfile::tempdir().unwrap();
        let arquivo = dir.path().join(anotacoes::NOME_ARQUIVO);
        let mut app = App::new(criar_artigos_exemplo(), Config::default(), None);
        app.carregar_anotacoes(arquivo.clone());

        app.alternar_status(); // Artigo 1: Lendo
        app.alternar_status(); // Artigo 1: Lido
        app.editando_tags = Some("Revisão, deep learning".into());
        app.confirmar_tags();
        app.proximo();
        app.alternar_status(); // Artigo 2: Lendo

        for c in "status:lido tag:rev".chars() {
            app.digitar_filtro(c);
        }
        assert_eq!(titulos_visiveis(&app), ["Artigo 1"]);
        app.limpar_filtro();
        for c in "status:l".chars() {
            app.digitar_filtro(c);
        }
        assert_eq!(titulos_visiveis(&app), ["Artigo 1", "Artigo 2"]);

        // reabrindo, as anotações voltam
        let mut outro = App::new(criar_artigos_exemplo(), Config::default(), None);
        outro.carregar_anotacoes(arquivo);
        assert_eq!(outro.artigos[0].anotacao.tags, ["Revisão", "deep-learning"]);
        assert_eq!(outro.artigos[1].anotacao.status, anotacoes::Status::Lendo);
    }

    #[test]
    fn arquivo_de_anotacoes_ilegivel_nao_e_sobrescrito() {
        let dir = tempfile::tempdir().unwrap();
        let arquivo = dir.path().join(anotacoes::NOME_ARQUIVO);
        std::fs::write(&arquivo, "lixo = [").unwrap();
        let mut app = App::new(criar_artigos_exemplo(), Config::default(), None);
        app.carregar_anotacoes(arquivo.clone());
        assert!(app.status_msg.contains("ilegível"));
        app.alternar_status();
        assert_eq!(std::fs::read_to_string(&arquivo).unwrap(), "lixo = [");
    }

    #[test]
    fn detecta_notas_existentes_e_cria_nota_do_selecionado() {
        let dir = tempfile::tempdir().unwrap();
        let notas = dir.path().join("notas");
        std::fs::create_dir_all(&notas).unwrap();
        std::fs::write(notas.join("Artigo 3.md"), "já existia").unwrap();

        let mut app = App::new(criar_artigos_exemplo(), Config::default(), None);
        app.definir_dir_notas(notas.clone());
        assert_eq!(app.com_nota.iter().copied().collect::<Vec<_>>(), [2]);

        let nota = app.preparar_nota().unwrap(); // selecionado: Artigo 1
        assert!(std::fs::read_to_string(nota).unwrap().starts_with("# Artigo 1"));
        assert!(app.com_nota.contains(&0));
        assert_eq!(std::fs::read_to_string(notas.join("Artigo 3.md")).unwrap(), "já existia");
    }
}
