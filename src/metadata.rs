//! Extração determinística de metadados bibliográficos de PDFs, 100% local.
//!
//! Ordem de confiança de cada campo:
//! 1. XMP (metadados embutidos pelas editoras: prism:doi, prism:publicationName…)
//! 2. Dicionário /Info (validado contra o texto da página 1, pois costuma ter lixo)
//! 3. Layout da página 1 via `pdf_texto` (tamanho de fonte: o maior bloco é o título, o seguinte são os autores)
//! 4. Padrões textuais (DOI, arXiv, ano, venue, resumo, palavras-chave, referências)

use lopdf::{Document, Object};
use crate::pdf_texto::{Linha, extrair_linhas, limpar};
use regex::Regex;
use std::path::Path;
use std::sync::LazyLock;

#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct PdfMetadata {
    pub titulo: Option<String>,
    pub autores: Option<String>, // separados por "; "
    pub ano: Option<String>,
    pub venue: Option<String>, // Conferência ou Journal
    pub volume: Option<String>,
    pub intervalo_paginas: Option<String>, // ex: "123–145"
    pub doi: Option<String>,
    pub arxiv: Option<String>,
    pub paginas: usize,
    pub n_referencias: Option<usize>,
    pub resumo: Option<String>,
    pub palavras_chave: Option<String>, // separadas por "; "
}

pub fn extrair_metadados(caminho: &Path) -> PdfMetadata {
    let Ok(doc) = Document::load(caminho) else {
        return PdfMetadata::default();
    };
    let n = doc.get_pages().len();
    let xmp = ler_xmp(&doc).unwrap_or_default();
    let info = ler_info(&doc);
    let linhas = if doc.is_encrypted() { Vec::new() } else { extrair_linhas(&doc, paginas_lidas(n as u32)) };
    montar(&xmp, &info, &linhas, n)
}

/// Junta todas as fontes. Separado de `extrair_metadados` para ser testável sem PDF.
fn montar(xmp: &str, info: &Info, linhas: &[Linha], paginas: usize) -> PdfMetadata {
    let pag1: Vec<&Linha> = linhas.iter().filter(|l| l.pagina == 1).collect();
    let pag12: String = linhas.iter().filter(|l| l.pagina <= 2).map(|l| l.texto.as_str()).collect::<Vec<_>>().join("\n");
    let texto1: String = pag1.iter().map(|l| l.texto.as_str()).collect::<Vec<_>>().join("\n");
    // Cabeçalho e rodapé da página 1: onde ficam venue, volume e ano
    let borda: String = pag1.iter().take(10).chain(pag1.iter().rev().take(8)).map(|l| l.texto.as_str()).collect::<Vec<_>>().join("\n");

    let cit = citacao(&borda).or_else(|| citacao(&texto1));
    let venue = xmp_prop(xmp, "prism:publicationName")
        .or_else(|| venue_do_subject(&info.assunto))
        .or_else(|| extrair_venue(&texto1, &borda, cit.as_ref()));
    // O nome do journal no cabeçalho (Elsevier) costuma ter a maior fonte da página
    let (titulo_layout, autores_layout) = titulo_e_autores_por_layout(&pag1, venue.as_deref());

    // Metadado tem texto mais limpo, mas só vence se concordar com o título visual (pode ser de um preprint/template)
    let titulo_meta = xmp_lista(xmp, "dc:title").into_iter().next().filter(|t| titulo_valido(t) && aparece_no_texto(t, &texto1))
        .or_else(|| info.titulo.clone().filter(|t| titulo_valido(t) && aparece_no_texto(t, &texto1)));
    let titulo = match (titulo_meta, titulo_layout) {
        (Some(m), Some(l)) if !similares(&m, &l) => Some(l),
        (m, l) => m.or(l),
    };

    let autores_xmp = xmp_lista(xmp, "dc:creator");
    let autores = (!autores_xmp.is_empty()).then(|| autores_xmp.join("; "))
        .or_else(|| info.autor.clone().filter(|a| autor_valido(a, &texto1)).map(|a| normalizar_lista(&a)))
        .or(autores_layout);

    let arxiv = RE_ARXIV.captures(&texto1).map(|c| c[1].to_string());

    let doi = xmp_prop(xmp, "prism:doi").or_else(|| xmp_prop(xmp, "pdfx:doi"))
        .or_else(|| achar_doi(xmp))
        .or_else(|| achar_doi(&info.todos))
        .or_else(|| achar_doi(&pag12));

    let venue = venue.or_else(|| arxiv.as_ref().map(|_| "arXiv".to_string()));

    let ano = ["prism:coverDate", "prism:publicationDate", "prism:coverDisplayDate"].iter()
        .find_map(|p| xmp_prop(xmp, p).and_then(|d| primeiro_ano(&d)))
        .or_else(|| ano_do_subject(&info.assunto))
        .or_else(|| cit.as_ref().and_then(|c| c.ano.clone()))
        .or_else(|| extrair_ano(&texto1, &borda))
        .or_else(|| arxiv.as_ref().and_then(|id| ano_valido(&format!("20{}", &id[..2]))));

    let volume = xmp_prop(xmp, "prism:volume")
        .or_else(|| cit.as_ref().and_then(|c| c.volume.clone()))
        .or_else(|| RE_VOLUME.captures(&borda).map(|c| c[1].to_string()));

    let intervalo_paginas = match (xmp_prop(xmp, "prism:startingPage"), xmp_prop(xmp, "prism:endingPage")) {
        (Some(a), Some(b)) => Some(format!("{a}–{b}")),
        (Some(a), None) => Some(a),
        _ => cit.as_ref().and_then(|c| c.paginas.clone()).or_else(|| RE_PP.captures(&borda).map(|c| format!("{}–{}", &c[1], &c[2]))),
    };

    let palavras_chave = info.palavras.clone().filter(|k| k.contains([',', ';', ' '])).map(|k| normalizar_lista(&k))
        .or_else(|| extrair_palavras_chave(&pag12));

    PdfMetadata {
        titulo,
        autores,
        ano,
        venue,
        volume,
        intervalo_paginas,
        doi,
        arxiv,
        paginas,
        n_referencias: contar_referencias(linhas),
        resumo: extrair_resumo(&pag12),
        palavras_chave,
    }
}

// ───────────────────────────── Regex ─────────────────────────────

macro_rules! re {
    ($name:ident, $pat:expr) => {
        static $name: LazyLock<Regex> = LazyLock::new(|| Regex::new($pat).unwrap());
    };
}

re!(RE_DOI, r"\b(10\.\d{4,9}/[-._;()/:A-Za-z0-9]+)");
re!(RE_ARXIV, r"(?i)arXiv:\s*(\d{4}\.\d{4,5}(?:v\d+)?|[a-z\-]+/\d{7})");
re!(RE_ANO, r"\b((?:19|20)\d{2})\b");
re!(RE_VOLUME, r"(?i)\bvol(?:ume)?\.?\s*(\d{1,4})\b");
re!(RE_PP, r"(?i)\bpp?\.\s*(\d{1,5})\s*[-–—]\s*(\d{1,5})\b");
re!(RE_LIXO_TITULO, r"(?i)\bjournal\b|\bvolume\b|\bvol\.|\bdoi\b|https?:|www\.|©|copyright|issn|received|contents lists|available online|arxiv|preprint|elsevier|springer|proceedings|licen[cs]e|homepage|\bpage \d|research article|original article|uncorrected|author proof|public access|author manuscript|accepted manuscript|^\s*article\s*$");
re!(RE_AFILIACAO, r"(?i)^\s*(?:\d+|[a-z])\s|univ|cnrs|inserm|department|departamento|institut|school|college|laborat|hospital|cent(?:er|re)|faculdade|faculty|@|abstract|resumo|received|a r t i c l e|keywords|correspond|e-?mail|\binc\b|\bltd\b|gmbh|street|road|avenue|^\s*(?:for|on behalf of) the\b");
re!(RE_HEADING_REFS, r"(?i)^\s*(?:\d+\.?\s*|[IVX]+\.\s*)?(?:references|bibliography|referências(?: bibliográficas)?|literature cited|works cited|reference list|cited literature)\s*:?(?:\s+\d{1,4})?\s*$");
re!(RE_FIM_REFS, r"(?i)^\s*(?:[A-Z]\.?\s+)?(?:appendix|apêndice|anexo|supplementary (?:material|information)|supporting information)\b");
re!(RE_REF_COLCHETE, r"^\s*\[(\d{1,4})\]");
re!(RE_REF_PONTO, r"^\s*(\d{1,4})\.\s+\S");
re!(RE_REF_AUTOR_ANO, r"^\s*(?:(?:de|da|do|dos|van|von|der|den|di|du|le|la)\s+)*[\p{Lu}][^\s,]+(?:\s[\p{Lu}][^\s,]+)?,\s+(?:[\p{Lu}]\.|[\p{Lu}][\p{L}]+)|^\s*(?:[\p{Lu}][^\s,]+\s){1,3}[\p{Lu}]{1,3}[,.]\s");
re!(RE_RESUMO, r"(?i)^\s*(?:abstract|a b s t r a c t|resumo|summary)\b\s*[.:—–-]?\s*(.*)$");
re!(RE_FIM_RESUMO, r"(?i)^\s*(?:keywords|key words|index terms|palavras-chave|ccs concepts|(?:1|i)\.?\s+introdu|introdu[cç]|©|\(c\)\s*\d{4}|a r t i c l e)");
re!(RE_PALAVRAS, r"(?i)^\s*(?:keywords|key words|index terms|palavras-chave)\s*[:—–.-]?\s*(.*)$");

// ───────────────────────────── Fontes de metadados ─────────────────────────────

#[derive(Debug, Default)]
struct Info {
    titulo: Option<String>,
    autor: Option<String>,
    assunto: String,
    palavras: Option<String>,
    todos: String, // todos os valores, para busca de DOI
}

fn ler_info(doc: &Document) -> Info {
    let dict = doc.trailer.get(b"Info").ok()
        .and_then(|o| doc.dereference(o).ok())
        .and_then(|(_, o)| o.as_dict().ok());
    let Some(dict) = dict else { return Info::default() };
    let campo = |k: &[u8]| {
        dict.get(k).ok().and_then(|o| doc.dereference(o).ok()).map(|(_, o)| string_pdf(o)).filter(|s| !s.is_empty())
    };
    let todos = dict.iter().filter_map(|(_, o)| doc.dereference(o).ok()).map(|(_, o)| string_pdf(o)).collect::<Vec<_>>().join("\n");
    Info {
        titulo: campo(b"Title").map(|s| limpar(&s)),
        autor: campo(b"Author").map(|s| limpar(&s)),
        assunto: campo(b"Subject").map(|s| limpar(&s)).unwrap_or_default(),
        palavras: campo(b"Keywords"),
        todos,
    }
}

/// Converte string PDF (UTF-16BE com BOM, UTF-8 ou PDFDocEncoding≈Latin-1)
fn string_pdf(obj: &Object) -> String {
    let Object::String(bytes, _) = obj else { return String::new() };
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let w: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&w).trim().to_string();
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => s.trim().to_string(),
        Err(_) => bytes.iter().map(|&b| pdfdoc_char(b)).collect::<String>().trim().to_string(),
    }
}

/// PDFDocEncoding = Latin-1, exceto 0x80–0x9F (aspas tipográficas, travessões…)
fn pdfdoc_char(b: u8) -> char {
    const ALTO: [char; 32] = ['•', '†', '‡', '…', '—', '–', 'ƒ', '⁄', '‹', '›', '−', '‰', '„', '“', '”', '‘',
        '’', '‚', '™', 'ﬁ', 'ﬂ', 'Ł', 'Œ', 'Š', 'Ÿ', 'Ž', 'ı', 'ł', 'œ', 'š', 'ž', '\u{FFFD}'];
    if (0x80..0xA0).contains(&b) { ALTO[(b - 0x80) as usize] } else { b as char }
}

fn ler_xmp(doc: &Document) -> Option<String> {
    let obj = doc.catalog().ok()?.get(b"Metadata").ok()?;
    let stream = doc.dereference(obj).ok()?.1.as_stream().ok()?;
    let bytes = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// Propriedade XMP simples, como elemento (<p>v</p>) ou atributo (p="v")
fn xmp_prop(xmp: &str, prop: &str) -> Option<String> {
    let p = regex::escape(prop);
    let re = Regex::new(&format!(r#"(?s)<{p}>([^<]*)</{p}>|\b{p}="([^"]*)""#)).ok()?;
    let c = re.captures(xmp)?;
    let v = decodificar_xml(c.get(1).or(c.get(2))?.as_str());
    let v = v.trim().trim_start_matches("doi:").trim();
    (!v.is_empty()).then(|| v.to_string())
}

/// Itens <rdf:li> de uma propriedade XMP em lista (dc:title, dc:creator)
fn xmp_lista(xmp: &str, prop: &str) -> Vec<String> {
    let p = regex::escape(prop);
    let Ok(re) = Regex::new(&format!(r"(?s)<{p}>(.*?)</{p}>")) else { return vec![] };
    static LI: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<rdf:li[^>]*>(.*?)</rdf:li>").unwrap());
    re.captures(xmp).map(|c| {
        LI.captures_iter(&c[1]).map(|li| limpar(&decodificar_xml(&li[1]))).filter(|s| !s.is_empty()).collect()
    }).unwrap_or_default()
}

fn decodificar_xml(s: &str) -> String {
    static ENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"&(#x[0-9a-fA-F]+|#\d+|amp|lt|gt|quot|apos);").unwrap());
    ENT.replace_all(s, |c: &regex::Captures| match &c[1] {
        "amp" => "&".into(),
        "lt" => "<".into(),
        "gt" => ">".into(),
        "quot" => "\"".into(),
        "apos" => "'".into(),
        e => {
            let n = e.strip_prefix("#x").map(|h| u32::from_str_radix(h, 16)).unwrap_or_else(|| e[1..].parse());
            n.ok().and_then(char::from_u32).map(String::from).unwrap_or_default()
        }
    }).into_owned()
}

// ───────────────────────────── Layout (fonte/posição) ─────────────────────────────

/// Lê as 2 primeiras páginas (título, resumo) e as 25 últimas (referências).
// shortcut: livros com bibliografia longa podem ter referências antes das 25 últimas páginas; aumentar se importar.
fn paginas_lidas(n: u32) -> impl Iterator<Item = u32> {
    (1..=n.min(2)).chain(n.saturating_sub(24).max(3)..=n)
}

/// Título = bloco de linhas consecutivas com a maior fonte da metade superior da página 1.
/// Autores = linhas logo após o título, antes de afiliações/resumo.
fn titulo_e_autores_por_layout(pag1: &[&Linha], venue: Option<&str>) -> (Option<String>, Option<String>) {
    let mut corpo: Vec<f64> = pag1.iter().map(|l| l.fonte).collect();
    corpo.sort_by(f64::total_cmp);
    let mediana = corpo.get(corpo.len() / 2).copied().unwrap_or(0.0);

    // Blocos de linhas consecutivas com a mesma fonte, sem linhas de lixo
    let mut blocos: Vec<(usize, usize)> = Vec::new(); // [inicio, fim)
    let mut i = 0;
    while i < pag1.len() {
        let mut j = i + 1;
        while j < pag1.len() && j - i < 5 && (pag1[j].fonte - pag1[i].fonte).abs() < 0.3 && !RE_LIXO_TITULO.is_match(&pag1[j].texto) {
            j += 1;
        }
        blocos.push((i, j));
        i = j;
    }

    let candidato = blocos.iter().filter(|&&(a, b)| {
        let texto = juntar(&pag1[a..b]);
        a < pag1.len().max(1) * 2 / 3
            && pag1[a].fonte > mediana * 1.15
            && !RE_LIXO_TITULO.is_match(&texto)
            && !venue.is_some_and(|v| similares(v, &texto))
            && texto.split_whitespace().filter(|w| w.chars().count() >= 3).count() >= 2
            && texto.chars().filter(|c| c.is_alphabetic()).count() >= 10
    }).max_by(|x, y| pag1[x.0].fonte.total_cmp(&pag1[y.0].fonte).then(y.0.cmp(&x.0)));

    let Some(&(a, b)) = candidato else { return (None, None) };
    let titulo = juntar(&pag1[a..b]).trim_end_matches(['*', '†', '‡', '∗', '⋆']).trim().to_string();

    let linhas_autores: Vec<&str> = pag1[b..].iter().take(8)
        .take_while(|l| !RE_AFILIACAO.is_match(&l.texto) && l.texto.len() < 250)
        .map(|l| l.texto.as_str()).collect();
    (Some(titulo), autores_da_linha(&linhas_autores.join(", ")))
}

fn juntar(linhas: &[&Linha]) -> String {
    desifenizar(&linhas.iter().map(|l| l.texto.as_str()).collect::<Vec<_>>())
}

/// Limpa marcadores de afiliação e separa nomes. Retorna None se não parecer lista de nomes.
fn autores_da_linha(s: &str) -> Option<String> {
    static SEP: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+(?:and|&|e)\s+|[,;|]").unwrap());
    const TITULOS: [&str; 10] = ["MD", "PhD", "MSc", "MS", "BSc", "MPH", "DPhil", "RN", "Jr", "Jr."];
    let sem_marcas: String = s.chars().filter(|c| !c.is_ascii_digit() && !"*†‡§¶#∗⁎✉⋆★".contains(*c)).collect();
    let pecas: Vec<String> = SEP.split(&sem_marcas).map(|p| {
        // remove letras sobrescritas de afiliação ("Silva a,b")
        p.split_whitespace().filter(|t| !(t.chars().count() == 1 && t.chars().all(char::is_lowercase))).collect::<Vec<_>>().join(" ")
    }).filter(|p| !p.is_empty() && !TITULOS.contains(&p.as_str())).collect();
    let nomes: Vec<&String> = pecas.iter().filter(|p| eh_nome(p)).collect();
    (!nomes.is_empty() && nomes.len() * 2 >= pecas.len()).then(|| nomes.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("; "))
}

fn eh_nome(p: &str) -> bool {
    const PARTICULAS: [&str; 12] = ["de", "da", "do", "dos", "das", "van", "von", "der", "den", "del", "la", "le"];
    let tokens: Vec<&str> = p.split_whitespace().collect();
    // "CNN Convolutional Neural Network" (lista de siglas) não é nome
    let sigla = tokens.first().is_some_and(|t| t.chars().count() >= 2 && t.chars().all(char::is_uppercase));
    !sigla && (2..=6).contains(&tokens.len())
        && tokens.iter().all(|t| t.chars().all(|c| c.is_alphabetic() || ".'’-".contains(c) || ('\u{300}'..='\u{36F}').contains(&c)))
        && tokens.iter().all(|t| PARTICULAS.contains(t) || t.chars().next().is_some_and(char::is_uppercase))
}

// ───────────────────────────── Campos textuais ─────────────────────────────

fn titulo_valido(titulo: &str) -> bool {
    let t = titulo.trim();
    let lower = t.to_lowercase();
    let ignorar = ["untitled", "microsoft word", ".tex", ".dvi", ".doc", ".pdf", "arxiv:", "pii:", "doi:", "paper title", "manuscript"];
    t.len() >= 4 && t.contains(' ') && !ignorar.iter().any(|bad| lower.contains(bad))
}

/// Metadado só é confiável se suas palavras aparecem na página 1 (ou se não há texto para comparar)
fn aparece_no_texto(valor: &str, texto: &str) -> bool {
    if texto.trim().is_empty() {
        return true;
    }
    let texto = normalizar_busca(texto);
    let palavras: Vec<String> = valor.split_whitespace().map(normalizar_busca).filter(|w| w.len() > 3).collect();
    palavras.is_empty() || palavras.iter().filter(|w| texto.contains(w.as_str())).count() * 10 >= palavras.len() * 6
}

/// Jaccard das palavras ≥ 0.8
fn similares(a: &str, b: &str) -> bool {
    use std::collections::HashSet;
    let pal = |s: &str| normalizar_busca(&s.replace(['-', '‐'], " ")).split_whitespace().map(String::from).collect::<HashSet<_>>();
    let (a, b) = (pal(a), pal(b));
    a.intersection(&b).count() * 5 >= a.union(&b).count() * 4
}

fn normalizar_busca(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric() || c.is_whitespace()).collect::<String>().to_lowercase()
}

fn autor_valido(autor: &str, texto1: &str) -> bool {
    let lower = autor.to_lowercase();
    let lixo = ["microsoft", "latex", "admin", "user", "owner", "unknown", "author", "pdf"];
    // Basta o sobrenome do primeiro autor aparecer na página 1
    let sobrenome = autor.split([',', ';']).next().and_then(|a| a.split_whitespace().last()).unwrap_or("");
    autor.contains(' ') && !lixo.iter().any(|l| lower.contains(l)) && aparece_no_texto(sobrenome, texto1)
}

fn achar_doi(texto: &str) -> Option<String> {
    let doi = RE_DOI.captures(texto)?.get(1)?.as_str().trim_end_matches(['.', ',', ';', ':']);
    // remove ")" de fechamento que não pertence ao DOI
    let mut d = doi.to_string();
    while d.ends_with(')') && d.matches(')').count() > d.matches('(').count() {
        d.pop();
    }
    Some(d)
}

fn ano_atual() -> i32 {
    let s = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    1970 + (s / 31_556_952) as i32
}

fn ano_valido(a: &str) -> Option<String> {
    a.parse::<i32>().ok().filter(|&y| (1900..=ano_atual() + 1).contains(&y)).map(|y| y.to_string())
}

fn primeiro_ano(s: &str) -> Option<String> {
    RE_ANO.captures_iter(s).find_map(|c| ano_valido(&c[1]))
}

/// Ano por ordem de confiança: data de publicação > copyright > aceite > linha de venue > mais frequente na borda.
fn extrair_ano(texto1: &str, borda: &str) -> Option<String> {
    static PADROES: LazyLock<Vec<Regex>> = LazyLock::new(|| {
        [
            r"(?i)(?:published(?: online)?|available online|publication date|publicado)[^\n]{0,40}?\b((?:19|20)\d{2})\b",
            r"(?i)(?:©|\(c\)|copyright)\s*(?:\d{4}\s*[-–]\s*)?((?:19|20)\d{2})\b",
            r"(?i)accepted[^\n]{0,40}?\b((?:19|20)\d{2})\b",
            r"\(((?:19|20)\d{2})\)",
            r"(?i)(?:proceedings of|conference|symposium|workshop)[^\n]{0,80}?\b((?:19|20)\d{2})\b",
        ].iter().map(|p| Regex::new(p).unwrap()).collect()
    });
    for (i, re) in PADROES.iter().enumerate() {
        // "(2019)" é comum em citações no corpo: só vale no cabeçalho/rodapé
        let alvo = if i == 3 { borda } else { texto1 };
        if let Some(a) = re.captures_iter(alvo).find_map(|c| ano_valido(&c[1])) {
            return Some(a);
        }
    }
    let mut freq: Vec<(String, usize)> = Vec::new();
    for a in RE_ANO.captures_iter(borda).filter_map(|c| ano_valido(&c[1])) {
        match freq.iter_mut().find(|(x, _)| *x == a) {
            Some(e) => e.1 += 1,
            None => freq.push((a, 1)),
        }
    }
    freq.into_iter().max_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0))).map(|(a, _)| a)
}

/// Subject do /Info no estilo Elsevier/Springer: "NeuroImage, 202 (2019) 116101. doi:10..."
fn venue_do_subject(assunto: &str) -> Option<String> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*([^,\d()]{4,}?)\s*[,.;]?\s*(?:\d|doi|\()").unwrap());
    if !(assunto.to_lowercase().contains("doi") || Regex::new(r"\((?:19|20)\d{2}\)").unwrap().is_match(assunto)) {
        return None;
    }
    RE.captures(assunto).map(|c| c[1].trim().to_string())
}

fn ano_do_subject(assunto: &str) -> Option<String> {
    venue_do_subject(assunto)?;
    primeiro_ano(assunto)
}

#[derive(Debug, Clone, PartialEq)]
struct Citacao {
    venue: String,
    ano: Option<String>,
    volume: Option<String>,
    paginas: Option<String>,
}

/// Linha de citação do cabeçalho/rodapé: "Hum Brain Mapp. 2022;43:3270–3282", "NeuroImage 203 (2019) 116108"
fn citacao(texto: &str) -> Option<Citacao> {
    static ANO_VOL: LazyLock<Regex> = LazyLock::new(|| Regex::new(
        r"([\p{Lu}][\p{L}’'&:\- ]{2,80}?)\.?®?,?\s+((?:19|20)\d{2})\s*[;,]\s*(\d+)(?:\s*\(\d+\))?(?:\s*:\s*(\w+(?:\s*[-–]\s*\w+)?))?"
    ).unwrap());
    static VOL_ANO: LazyLock<Regex> = LazyLock::new(|| Regex::new(
        r"(?m)^\s*([\p{Lu}][\p{L}’'&:\- ]{2,80}?)\s+(\d+|x+)\s*\(((?:19|20)\d{2}|\d{2}xx)\)\s*(\w+(?:\s*[-–]\s*\w+)?)?"
    ).unwrap());
    let ok = |s: Option<regex::Match>| s.map(|m| m.as_str().to_string()).filter(|v| !v.contains('x'));
    if let Some(c) = ANO_VOL.captures(texto) {
        return Some(Citacao { venue: c[1].trim().into(), ano: ano_valido(&c[2]), volume: ok(c.get(3)), paginas: ok(c.get(4)) });
    }
    VOL_ANO.captures(texto).map(|c| Citacao { venue: c[1].trim().into(), ano: ano_valido(&c[3]), volume: ok(c.get(2)), paginas: ok(c.get(4)) })
}

fn extrair_venue(texto1: &str, borda: &str, cit: Option<&Citacao>) -> Option<String> {
    // Elsevier: "Contents lists available at ScienceDirect" seguido do nome do journal
    static SCIDIRECT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)contents lists available at[^\n]*\n\s*([^\n]+)").unwrap());
    static FORTES: LazyLock<Regex> = LazyLock::new(|| Regex::new(
        r"(?i)(?:published (?:in|as)|accepted (?:at|in|to|for publication in)|presented at|to appear in|appeared in)\s*:?\s*(?:a conference paper at\s*)?(?:the\s+)?([^\n.,;]{4,100})|(Proceedings of [^\n]{4,150})"
    ).unwrap());
    static JOURNAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(
        r"(?im)^\s*((?:[\p{L}&\-]+\s+){0,4}(?:journal|transactions|letters|annals|revista|review|bulletin|magazine)\b[^\n]{0,80})$"
    ).unwrap());
    // Siglas exigem caixa exata e limite de palavra (antes "acl" casava com "oracle")
    static SIGLAS: LazyLock<Regex> = LazyLock::new(|| Regex::new(
        r"\b(NeurIPS|NIPS|ICML|ICLR|CVPR|ICCV|ECCV|AAAI|IJCAI|ACL|EMNLP|NAACL|SIGMOD|VLDB|KDD|SIGIR|WWW|CHI|ICSE|FSE|OSDI|SOSP|MICCAI|ICRA|IROS)\s*['’]?\s*(\d{2,4})?\b"
    ).unwrap());

    if let Some(c) = SCIDIRECT.captures(texto1) {
        return Some(c[1].trim().to_string());
    }
    if let Some(c) = cit {
        return Some(c.venue.clone());
    }
    if let Some(c) = FORTES.captures(texto1).filter(|c| !c[0].contains("final edited form")) {
        return Some(cortar_venue(c.get(1).or(c.get(2))?.as_str()));
    }
    if let Some(c) = JOURNAL.captures(borda).filter(|c| !c[1].to_lowercase().contains("homepage")) {
        return Some(cortar_venue(&c[1]));
    }
    SIGLAS.captures(borda).map(|c| c[0].trim().to_string())
}

/// Remove volume/ano/páginas que costumam seguir o nome do journal no cabeçalho
fn cortar_venue(s: &str) -> String {
    static CAUDA: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s*(?:,|\bvol\b|\bvolume\b|\bno\.|\(\d{4}\)|\s\d+\s*\().*$").unwrap());
    CAUDA.replace(s, "").trim().trim_end_matches([',', ';', ':', '.']).to_string()
}

fn extrair_resumo(texto: &str) -> Option<String> {
    let linhas: Vec<&str> = texto.lines().collect();
    let inicio = linhas.iter().position(|l| RE_RESUMO.is_match(l))?;
    let primeira = RE_RESUMO.captures(linhas[inicio])?.get(1).map_or("", |m| m.as_str());
    let mut partes = vec![primeira];
    let mut total = primeira.len();
    for l in &linhas[inicio + 1..] {
        if RE_FIM_RESUMO.is_match(l) || total > 3000 {
            break;
        }
        partes.push(l);
        total += l.len();
    }
    let r = desifenizar(&partes);
    (r.len() > 50).then_some(r)
}

fn extrair_palavras_chave(texto: &str) -> Option<String> {
    let linhas: Vec<&str> = texto.lines().collect();
    let i = linhas.iter().position(|l| RE_PALAVRAS.is_match(l))?;
    let resto = RE_PALAVRAS.captures(linhas[i])?.get(1).map_or("", |m| m.as_str()).trim().to_string();
    let bruto = if !resto.is_empty() {
        resto
    } else {
        // Elsevier: uma palavra-chave por linha
        linhas[i + 1..].iter().take(8).take_while(|l| l.len() < 60 && !RE_FIM_RESUMO.is_match(l) && !RE_RESUMO.is_match(l)).copied().collect::<Vec<_>>().join("; ")
    };
    let k = normalizar_lista(&bruto);
    (!k.is_empty()).then_some(k)
}

fn normalizar_lista(s: &str) -> String {
    s.split([';', ',', '·', '•', '|']).map(|p| p.trim().trim_end_matches('.')).filter(|p| !p.is_empty()).collect::<Vec<_>>().join("; ")
}

/// Conta entradas após o último cabeçalho "References". Tenta numeração [n] / n., depois estilo autor-ano.
fn contar_referencias(linhas: &[Linha]) -> Option<usize> {
    let inicio = linhas.iter().rposition(|l| RE_HEADING_REFS.is_match(&l.texto))? + 1;
    let secao: Vec<&Linha> = linhas[inicio..].iter()
        .take_while(|l| !(RE_FIM_REFS.is_match(&l.texto) && l.texto.len() < 60)).collect();

    // Numeração sequencial é robusta contra números soltos (páginas, volumes) no início de linha
    for re in [&*RE_REF_COLCHETE, &*RE_REF_PONTO] {
        let mut esperado = 1;
        for l in &secao {
            if re.captures(&l.texto).and_then(|c| c[1].parse::<usize>().ok()) == Some(esperado) {
                esperado += 1;
            }
        }
        if esperado > 3 {
            return Some(esperado - 1);
        }
    }

    // Autor-ano: nova entrada começa com "Sobrenome, X.", na margem da coluna (continuações têm recuo)
    // e depois de uma linha que fechou a entrada anterior
    // só posições x recorrentes contam como margem (ignora número de página, linhas soltas)
    // margens mudam entre páginas pares/ímpares: compara só dentro da mesma página
    let recorrente = |o: &Linha| secao.iter().filter(|p| p.pagina == o.pagina && (p.x - o.x).abs() < 1.).count() >= 3;
    let na_margem = |l: &Linha| !secao.iter().any(|o| o.pagina == l.pagina && o.x < l.x - 2. && o.x > l.x - 25. && recorrente(o));
    let tem_recuo = secao.iter().any(|l| !na_margem(l));
    let mut n = 0;
    let mut anterior_fechou = true;
    for l in &secao {
        // com recuo pendente a margem já delimita as entradas; sem ele, depende do fim da linha anterior
        let inicio = if tem_recuo { na_margem(l) } else { anterior_fechou };
        if inicio && RE_REF_AUTOR_ANO.is_match(&l.texto) {
            n += 1;
        }
        let t = l.texto.trim_end();
        anterior_fechou = t.ends_with(['.', ')']) || t.chars().last().is_some_and(|c| c.is_ascii_digit()) || t.contains("doi") || t.contains("http");
    }
    (n > 0).then_some(n)
}

// ───────────────────────────── Utilitários de texto ─────────────────────────────

/// Junta linhas desfazendo hifenização de fim de linha ("compu-" + "tation")
fn desifenizar(linhas: &[&str]) -> String {
    let mut out = String::new();
    for l in linhas.iter().map(|l| l.trim()).filter(|l| !l.is_empty()) {
        if out.ends_with('-') && l.chars().next().is_some_and(char::is_lowercase) {
            out.pop();
        } else if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(l);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(texto: &str, fonte: f64, pagina: u32) -> Linha {
        Linha { texto: texto.into(), fonte, pagina, x: 0. }
    }

    #[test]
    fn deve_validar_titulos_reais_e_rejeitar_lixo_de_editor() {
        assert!(!titulo_valido(""));
        assert!(!titulo_valido("Microsoft Word - artigo_final.docx"));
        assert!(!titulo_valido("Untitled"));
        assert!(!titulo_valido("jad-2019-0042"));
        assert!(titulo_valido("Attention Is All You Need"));
    }

    #[test]
    fn deve_extrair_doi_sem_pontuacao_final() {
        assert_eq!(achar_doi("DOI: 10.1145/3581783.3611902."), Some("10.1145/3581783.3611902".into()));
        assert_eq!(achar_doi("(see 10.1016/j.neuro.2019.01.001)"), Some("10.1016/j.neuro.2019.01.001".into()));
    }

    #[test]
    fn sigla_nao_casa_dentro_de_palavra() {
        assert_eq!(extrair_venue("", "Using an oracle for search", None), None);
        let t = "Published as a conference paper at ICLR 2021";
        assert_eq!(extrair_venue(t, t, None), Some("ICLR 2021".into()));
    }

    #[test]
    fn deve_extrair_venue_e_ano_de_proceedings() {
        let t = "Proceedings of the 2024 ACM SIGMOD International Conference\nFast Vector Search";
        assert_eq!(extrair_venue(t, t, None), Some("Proceedings of the 2024 ACM SIGMOD International Conference".into()));
        assert_eq!(extrair_ano(t, t), Some("2024".into()));
    }

    #[test]
    fn ano_de_copyright_vence_ano_citado_no_corpo() {
        let t = "as shown by Smith (1998)\n© 2019 Elsevier Inc. All rights reserved.";
        assert_eq!(extrair_ano(t, ""), Some("2019".into()));
    }

    #[test]
    fn deve_ler_linha_de_citacao() {
        let c = citacao("3270 Hum Brain Mapp. 2022;43:3270–3282.").unwrap();
        assert_eq!((c.venue.as_str(), c.ano.as_deref(), c.volume.as_deref(), c.paginas.as_deref()), ("Hum Brain Mapp", Some("2022"), Some("43"), Some("3270–3282")));
        let c = citacao("NeuroImage 203 (2019) 116108").unwrap();
        assert_eq!((c.venue.as_str(), c.ano.as_deref(), c.volume.as_deref()), ("NeuroImage", Some("2019"), Some("203")));
        let c = citacao("Journal of Alzheimer’s Disease xx (20xx) x–xx").unwrap();
        assert_eq!((c.venue.as_str(), c.ano, c.volume), ("Journal of Alzheimer’s Disease", None, None));
    }

    #[test]
    fn deve_ler_xmp() {
        let xmp = r#"<rdf:Description prism:doi="10.1/abc" prism:coverDate="2019-11-01">
            <dc:title><rdf:Alt><rdf:li xml:lang="x-default">Deep &amp; Wide</rdf:li></rdf:Alt></dc:title>
            <dc:creator><rdf:Seq><rdf:li>Ana Silva</rdf:li><rdf:li>John Doe</rdf:li></rdf:Seq></dc:creator>
            <prism:publicationName>NeuroImage</prism:publicationName></rdf:Description>"#;
        let m = montar(xmp, &Info::default(), &[], 10);
        assert_eq!(m.titulo.as_deref(), Some("Deep & Wide"));
        assert_eq!(m.autores.as_deref(), Some("Ana Silva; John Doe"));
        assert_eq!(m.doi.as_deref(), Some("10.1/abc"));
        assert_eq!(m.ano.as_deref(), Some("2019"));
        assert_eq!(m.venue.as_deref(), Some("NeuroImage"));
    }

    #[test]
    fn deve_inferir_titulo_e_autores_pelo_layout() {
        let linhas = vec![
            l("Journal of Testing 12 (2020) 1-10", 8.0, 1),
            l("A Deterministic Approach to", 17.0, 1),
            l("Metadata Extraction", 17.0, 1),
            l("Ana Silva1,*, John van Doe2 and Mary Smith1", 11.0, 1),
            l("1 Department of Computing, University X", 9.0, 1),
            l("Abstract", 10.0, 1),
            l("We propose a simple method for extracting metadata from scientific articles without any network.", 10.0, 1),
            l("Keywords: metadata, PDF; extraction", 10.0, 1),
            l("1 Introduction", 12.0, 1),
            l("body text body text body text", 10.0, 1),
            l("body text body text body text", 10.0, 1),
            l("body text body text body text", 10.0, 1),
        ];
        let m = montar("", &Info::default(), &linhas, 10);
        assert_eq!(m.titulo.as_deref(), Some("A Deterministic Approach to Metadata Extraction"));
        assert_eq!(m.autores.as_deref(), Some("Ana Silva; John van Doe; Mary Smith"));
        assert_eq!(m.venue.as_deref(), Some("Journal of Testing"));
        assert_eq!(m.ano.as_deref(), Some("2020"));
        assert_eq!(m.palavras_chave.as_deref(), Some("metadata; PDF; extraction"));
        assert!(m.resumo.unwrap().starts_with("We propose"));
    }

    #[test]
    fn info_lixo_nao_sobrescreve_layout() {
        let linhas = vec![l("Real Paper Title Here", 18.0, 1), l("body", 10.0, 1), l("body", 10.0, 1)];
        let info = Info { titulo: Some("Template for IEEE Conference Papers".into()), ..Default::default() };
        assert_eq!(montar("", &info, &linhas, 1).titulo.as_deref(), Some("Real Paper Title Here"));
    }

    #[test]
    fn deve_contar_referencias_numeradas_e_autor_ano() {
        let numeradas = vec![
            l("References", 12.0, 9), l("[1] A. Silva. Foo. 2020.", 8.0, 9), l("continuação 12 da linha", 8.0, 9),
            l("[2] B. Doe. Bar. 2021.", 8.0, 9), l("[3] C. Roe. Baz. 2019.", 8.0, 10), l("[4] D. Poe. Qux. 2018.", 8.0, 10),
        ];
        assert_eq!(contar_referencias(&numeradas), Some(4));

        let autor_ano = vec![
            l("References", 12.0, 9),
            l("Silva, A., Doe, B., 2020. A long title that wraps", 8.0, 9),
            l("Smith, J. continues here. NeuroImage 1, 1-2.", 8.0, 9),
            l("Doe, J., 2019. Another. Brain 3, 4.", 8.0, 9),
            l("Appendix A", 12.0, 10),
            l("Roe, K., 2018. Not a ref.", 8.0, 10),
        ];
        assert_eq!(contar_referencias(&autor_ano), Some(2));
    }
}
