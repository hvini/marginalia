//! Extração de texto posicionado sobre o `lopdf`: interpreta os operadores de texto do
//! content stream (Tf, Tm, Td, TJ, Do…) e devolve linhas com o tamanho de fonte predominante.
//! Não imprime nada e não usa `unwrap` em dados do PDF.

use lopdf::{Dictionary, Document, Object, ObjectId, content::Content, content::Operation};
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Debug, Clone, PartialEq)]
pub struct Linha {
    pub texto: String,
    pub fonte: f64, // tamanho de fonte predominante na linha, em pontos
    pub pagina: u32,
    pub x: f64, // início horizontal da linha (detecta recuo)
}

/// Extrai as linhas das páginas pedidas (numeração a partir de 1), na ordem do content stream.
pub fn extrair_linhas(doc: &Document, paginas: impl IntoIterator<Item = u32>) -> Vec<Linha> {
    let todas = doc.get_pages();
    let mut interp = Interpretador { doc, cache: HashMap::new(), trechos: Vec::new() };
    let mut linhas = Vec::new();
    for p in paginas {
        let Some(&id) = todas.get(&p) else { continue };
        let Ok(dados) = doc.get_page_content(id) else { continue };
        let Ok(conteudo) = Content::decode(&dados) else { continue };
        let recursos = recursos_pagina(doc, id);
        interp.executar(&conteudo.operations, &recursos, Estado::default(), 0);
        montar_linhas(std::mem::take(&mut interp.trechos), p, &mut linhas);
    }
    linhas
}

/// Remove ligaduras, hífen condicional, caracteres de controle e espaços repetidos.
pub(crate) fn limpar(s: &str) -> String {
    let s = s.replace('\u{FB01}', "fi").replace('\u{FB02}', "fl").replace('\u{FB00}', "ff")
        .replace('\u{FB03}', "ffi").replace('\u{FB04}', "ffl").replace('\u{00AD}', "")
        .replace(|c: char| c.is_control(), " ");
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ───────────────────────────── Interpretador ─────────────────────────────

type Matriz = [f64; 6];
const IDENTIDADE: Matriz = [1., 0., 0., 1., 0., 0.];

fn mul(m: &Matriz, n: &Matriz) -> Matriz {
    [
        m[0] * n[0] + m[1] * n[2],
        m[0] * n[1] + m[1] * n[3],
        m[2] * n[0] + m[3] * n[2],
        m[2] * n[1] + m[3] * n[3],
        m[4] * n[0] + m[5] * n[2] + n[4],
        m[4] * n[1] + m[5] * n[3] + n[5],
    ]
}

fn translacao(x: f64, y: f64) -> Matriz {
    [1., 0., 0., 1., x, y]
}

#[derive(Clone)]
struct Estado {
    ctm: Matriz,
    fonte: Option<Rc<Fonte>>,
    tam: f64,
    tc: f64,
    tw: f64,
    th: f64,
    tl: f64,
    rise: f64,
}

impl Default for Estado {
    fn default() -> Self {
        Self { ctm: IDENTIDADE, fonte: None, tam: 0., tc: 0., tw: 0., th: 1., tl: 0., rise: 0. }
    }
}

/// Texto de um Tj (ou de um elemento de TJ) com posição em espaço de dispositivo
struct Trecho {
    texto: String,
    x: f64,
    y: f64,
    fim: f64,
    tam: f64,
    rotacionado: bool,
}

struct Interpretador<'a> {
    doc: &'a Document,
    cache: HashMap<ObjectId, Rc<Fonte>>,
    trechos: Vec<Trecho>,
}

impl Interpretador<'_> {
    fn executar(&mut self, ops: &[Operation], recursos: &Dictionary, mut est: Estado, prof: u8) {
        let mut pilha: Vec<Estado> = Vec::new();
        let (mut tm, mut tlm) = (IDENTIDADE, IDENTIDADE);
        for op in ops {
            let n = |i: usize| op.operands.get(i).and_then(num).unwrap_or(0.);
            match op.operator.as_str() {
                "q" => pilha.push(est.clone()),
                "Q" => {
                    if let Some(e) = pilha.pop() {
                        est = e;
                    }
                }
                "cm" => est.ctm = mul(&[n(0), n(1), n(2), n(3), n(4), n(5)], &est.ctm),
                "BT" => (tm, tlm) = (IDENTIDADE, IDENTIDADE),
                "Tf" => {
                    est.fonte = op.operands.first().and_then(|o| o.as_name().ok()).and_then(|nome| self.fonte(recursos, nome));
                    est.tam = n(1);
                }
                "Tc" => est.tc = n(0),
                "Tw" => est.tw = n(0),
                "Tz" => est.th = n(0) / 100.,
                "TL" => est.tl = n(0),
                "Ts" => est.rise = n(0),
                "Td" | "TD" => {
                    if op.operator == "TD" {
                        est.tl = -n(1);
                    }
                    tlm = mul(&translacao(n(0), n(1)), &tlm);
                    tm = tlm;
                }
                "Tm" => {
                    tlm = [n(0), n(1), n(2), n(3), n(4), n(5)];
                    tm = tlm;
                }
                "T*" | "'" | "\"" => {
                    if op.operator == "\"" {
                        (est.tw, est.tc) = (n(0), n(1));
                    }
                    tlm = mul(&translacao(0., -est.tl), &tlm);
                    tm = tlm;
                    if op.operator != "T*" {
                        self.mostrar(op.operands.last(), &est, &mut tm);
                    }
                }
                "Tj" => self.mostrar(op.operands.first(), &est, &mut tm),
                "TJ" => {
                    let Some(Object::Array(itens)) = op.operands.first() else { continue };
                    for item in itens {
                        match num(item) {
                            Some(v) => tm = mul(&translacao(-v / 1000. * est.tam * est.th, 0.), &tm),
                            None => self.mostrar(Some(item), &est, &mut tm),
                        }
                    }
                }
                // Form XObjects (cabeçalhos de editora às vezes ficam aqui); profundidade limitada contra ciclos
                "Do" if prof < 5 => {
                    if let Some(nome) = op.operands.first().and_then(|o| o.as_name().ok()) {
                        self.form(nome, recursos, &est, prof);
                    }
                }
                _ => {}
            }
        }
    }

    fn mostrar(&mut self, o: Option<&Object>, est: &Estado, tm: &mut Matriz) {
        let (Some(Object::String(bytes, _)), Some(f)) = (o, est.fonte.as_ref()) else { return };
        let escala = [est.tam * est.th, 0., 0., est.tam, 0., est.rise];
        let trm0 = mul(&mul(&escala, tm), &est.ctm);
        let mut texto = String::new();
        for c in f.codigos(bytes) {
            if let Some(s) = f.mapa.get(&c) {
                texto.push_str(s);
            }
            let w = f.larguras.get(&c).copied().unwrap_or(f.largura_padrao) / 1000.;
            let espaco = if f.bytes == 1 && c == 32 { est.tw } else { 0. };
            *tm = mul(&translacao((w * est.tam + est.tc + espaco) * est.th, 0.), tm);
        }
        let trm1 = mul(&mul(&escala, tm), &est.ctm);
        self.trechos.push(Trecho {
            texto,
            x: trm0[4],
            y: trm0[5],
            fim: trm1[4],
            tam: trm0[2].hypot(trm0[3]),
            rotacionado: trm0[1].abs() > trm0[0].abs(),
        });
    }

    fn form(&mut self, nome: &[u8], recursos: &Dictionary, est: &Estado, prof: u8) {
        let doc = self.doc;
        let Some(xobjs) = recursos.get(b"XObject").ok().and_then(|o| dict(doc, o)) else { return };
        let Some(stream) = xobjs.get(nome).ok().and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_stream().ok()) else { return };
        if stream.dict.get(b"Subtype").and_then(Object::as_name).ok() != Some(b"Form".as_slice()) {
            return;
        }
        let dados = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
        let Ok(conteudo) = Content::decode(&dados) else { return };
        let mut e = est.clone();
        if let Some(m) = stream.dict.get(b"Matrix").ok().and_then(|o| arr(doc, o)).filter(|m| m.len() == 6) {
            let v: Vec<f64> = m.iter().map(|x| num(x).unwrap_or(0.)).collect();
            e.ctm = mul(&[v[0], v[1], v[2], v[3], v[4], v[5]], &e.ctm);
        }
        let proprios = stream.dict.get(b"Resources").ok().and_then(|o| dict(doc, o));
        self.executar(&conteudo.operations, proprios.unwrap_or(recursos), e, prof + 1);
    }

    fn fonte(&mut self, recursos: &Dictionary, nome: &[u8]) -> Option<Rc<Fonte>> {
        let doc = self.doc;
        let obj = dict(doc, recursos.get(b"Font").ok()?)?.get(nome).ok()?;
        match obj {
            Object::Reference(id) => {
                if let Some(f) = self.cache.get(id) {
                    return Some(f.clone());
                }
                let f = Rc::new(Fonte::carregar(doc, doc.get_dictionary(*id).ok()?));
                self.cache.insert(*id, f.clone());
                Some(f)
            }
            Object::Dictionary(d) => Some(Rc::new(Fonte::carregar(doc, d))),
            _ => None,
        }
    }
}

/// Junta Font e XObject da página e dos nós ancestrais (recursos herdados)
fn recursos_pagina(doc: &Document, id: ObjectId) -> Dictionary {
    let (direto, herdados) = doc.get_page_resources(id).unwrap_or((None, Vec::new()));
    let (mut fontes, mut xobjs) = (Dictionary::new(), Dictionary::new());
    for r in direto.into_iter().chain(herdados.iter().filter_map(|i| doc.get_dictionary(*i).ok())) {
        for (chave, destino) in [(b"Font".as_slice(), &mut fontes), (b"XObject".as_slice(), &mut xobjs)] {
            if let Some(d) = r.get(chave).ok().and_then(|o| dict(doc, o)) {
                for (k, v) in d.iter() {
                    if !destino.has(k) {
                        destino.set(k.clone(), v.clone());
                    }
                }
            }
        }
    }
    let mut r = Dictionary::new();
    r.set("Font", fontes);
    r.set("XObject", xobjs);
    r
}

fn num(o: &Object) -> Option<f64> {
    match o {
        Object::Integer(i) => Some(*i as f64),
        Object::Real(r) => Some(*r as f64),
        _ => None,
    }
}

fn dict<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Dictionary> {
    doc.dereference(o).ok()?.1.as_dict().ok()
}

fn arr<'a>(doc: &'a Document, o: &'a Object) -> Option<&'a Vec<Object>> {
    doc.dereference(o).ok()?.1.as_array().ok()
}

// ───────────────────────────── Linhas ─────────────────────────────

/// Agrupa trechos em linhas: quebra quando muda a linha de base ou o texto volta para a esquerda.
/// Sobrescritos (afiliações "Silva¹ᵃ") ficam na mesma linha, mas separados por espaço.
fn montar_linhas(trechos: Vec<Trecho>, pagina: u32, saida: &mut Vec<Linha>) {
    let mut texto = String::new();
    let mut tamanhos: Vec<(f64, usize)> = Vec::new();
    let mut x0 = 0.;
    let fechar = |texto: &mut String, tamanhos: &mut Vec<(f64, usize)>, x: f64, saida: &mut Vec<Linha>| {
        let t = limpar(texto);
        if !t.is_empty() {
            saida.push(Linha { texto: t, fonte: moda(tamanhos), pagina, x });
        }
        texto.clear();
        tamanhos.clear();
    };
    let (mut ult_y, mut ult_fim, mut ult_tam, mut base_tam, mut base_y) = (0., 0., 0., 0., 0.);
    let mut ant_sobrescrito = false;
    for t in trechos.into_iter().filter(|t| !t.texto.is_empty()) {
        if t.rotacionado {
            // texto vertical (carimbo do arXiv na margem) vira uma linha própria
            fechar(&mut texto, &mut tamanhos, x0, saida);
            texto.push_str(&t.texto);
            tamanhos.push((t.tam, 1));
            fechar(&mut texto, &mut tamanhos, t.x, saida);
            continue;
        }
        let r = t.tam.max(ult_tam);
        let nova = texto.is_empty() || (t.y - ult_y).abs() > r * 0.6 || t.x < ult_fim - r * 3.;
        if nova {
            fechar(&mut texto, &mut tamanhos, x0, saida);
            (base_tam, base_y, x0) = (t.tam, t.y, t.x);
        }
        let sobrescrito = t.tam < base_tam * 0.85 && t.y > base_y + t.tam * 0.2;
        if !nova && (t.x > ult_fim + r * 0.15 || (sobrescrito && !ant_sobrescrito)) {
            texto.push(' ');
        }
        texto.push_str(&t.texto);
        tamanhos.push((t.tam, t.texto.chars().filter(|c| !c.is_whitespace()).count()));
        if !sobrescrito {
            (base_tam, base_y) = (t.tam, t.y);
        }
        (ant_sobrescrito, ult_y, ult_fim, ult_tam) = (sobrescrito, t.y, t.fim, t.tam);
    }
    fechar(&mut texto, &mut tamanhos, x0, saida);
}

/// Tamanho com mais caracteres na linha, agrupado em 0.5pt
fn moda(v: &[(f64, usize)]) -> f64 {
    let mut cont: HashMap<i64, usize> = HashMap::new();
    for &(t, n) in v {
        *cont.entry((t * 2.).round() as i64).or_default() += n;
    }
    cont.into_iter().max_by_key(|&(k, n)| (n, k)).map_or(0., |(k, _)| k as f64 / 2.)
}

// ───────────────────────────── Fontes ─────────────────────────────

struct Fonte {
    mapa: HashMap<u32, String>, // código → Unicode
    bytes: usize,               // bytes por código (1 simples, 2 composta)
    larguras: HashMap<u32, f64>, // em 1/1000 de em
    largura_padrao: f64,
}

impl Fonte {
    fn codigos<'a>(&self, s: &'a [u8]) -> impl Iterator<Item = u32> + 'a {
        s.chunks(self.bytes).map(|c| c.iter().fold(0u32, |a, &b| (a << 8) | b as u32))
    }

    fn carregar(doc: &Document, d: &Dictionary) -> Fonte {
        let nome = |k: &[u8]| d.get(k).and_then(Object::as_name).ok();
        let composta = nome(b"Subtype") == Some(b"Type0".as_slice());
        let mut f = Fonte {
            mapa: HashMap::new(),
            bytes: if composta { 2 } else { 1 },
            larguras: HashMap::new(),
            largura_padrao: if composta { 1000. } else { 500. },
        };

        if composta {
            // shortcut: só CMaps Identity (2 bytes); CMaps CJK predefinidos ficam sem texto, implementar se aparecerem
            let desc = d.get(b"DescendantFonts").ok().and_then(|o| arr(doc, o)).and_then(|a| a.first()).and_then(|o| dict(doc, o));
            if let Some(desc) = desc {
                f.largura_padrao = desc.get(b"DW").ok().and_then(num).unwrap_or(1000.);
                if let Some(w) = desc.get(b"W").ok().and_then(|o| arr(doc, o)) {
                    ler_larguras_cid(doc, w, &mut f.larguras);
                }
            }
        } else {
            let enc = d.get(b"Encoding").ok().and_then(|o| doc.dereference(o).ok()).map(|(_, o)| o);
            let base = match enc {
                Some(Object::Name(n)) => Some(n.as_slice()),
                Some(Object::Dictionary(e)) => e.get(b"BaseEncoding").and_then(Object::as_name).ok(),
                _ => None,
            };
            let padrao = base == Some(b"StandardEncoding".as_slice()) || (base.is_none() && nome(b"Subtype") == Some(b"Type1".as_slice()));
            for c in 0..=255u8 {
                if let Some(s) = codigo_base(c, padrao) {
                    f.mapa.insert(c as u32, s);
                }
            }
            if let Some(Object::Dictionary(e)) = enc
                && let Some(difs) = e.get(b"Differences").ok().and_then(|o| arr(doc, o))
            {
                let mut c = 0u32;
                for o in difs {
                    match o {
                        Object::Integer(i) => c = *i as u32,
                        Object::Name(n) => {
                            match glifo(&String::from_utf8_lossy(n)) {
                                Some(s) => f.mapa.insert(c, s),
                                None => f.mapa.remove(&c),
                            };
                            c += 1;
                        }
                        _ => {}
                    }
                }
            }
            let primeiro = d.get(b"FirstChar").ok().and_then(num).unwrap_or(0.) as u32;
            if let Some(ws) = d.get(b"Widths").ok().and_then(|o| arr(doc, o)) {
                for (i, w) in ws.iter().enumerate() {
                    if let Some(w) = doc.dereference(w).ok().and_then(|(_, o)| num(o)) {
                        f.larguras.insert(primeiro + i as u32, w);
                    }
                }
            }
        }

        // ToUnicode é a fonte mais precisa: sobrescreve o que veio da codificação
        let tu = d.get(b"ToUnicode").ok().and_then(|o| doc.dereference(o).ok()).and_then(|(_, o)| o.as_stream().ok());
        if let Some(s) = tu {
            let dados = s.decompressed_content().unwrap_or_else(|_| s.content.clone());
            let (mapa, bytes) = ler_tounicode(&dados);
            if let Some(b) = bytes.filter(|b| (1..=4).contains(b)) {
                f.bytes = b;
            }
            f.mapa.extend(mapa);
        }
        f
    }
}

fn ler_larguras_cid(doc: &Document, w: &[Object], larguras: &mut HashMap<u32, f64>) {
    let item = |i: usize| w.get(i).and_then(|o| doc.dereference(o).ok()).map(|(_, o)| o);
    let mut i = 0;
    while let Some(c) = item(i).and_then(num) {
        match item(i + 1) {
            Some(Object::Array(ws)) => {
                for (k, x) in ws.iter().enumerate() {
                    if let Some(x) = num(x) {
                        larguras.insert(c as u32 + k as u32, x);
                    }
                }
                i += 2;
            }
            Some(o) => {
                let (Some(c2), Some(v)) = (num(o), item(i + 2).and_then(num)) else { return };
                for k in c as u32..=(c2 as u32).min(c as u32 + 65_535) {
                    larguras.insert(k, v);
                }
                i += 3;
            }
            None => return,
        }
    }
}

enum Tok {
    Hex(Vec<u8>),
    Abre,
    Fecha,
}

fn tokens(s: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        match c {
            '<' => {
                let hex: String = it.by_ref().take_while(|&c| c != '>').filter(char::is_ascii_hexdigit).collect();
                let hex = if hex.len() % 2 == 1 { hex + "0" } else { hex };
                out.push(Tok::Hex((0..hex.len()).step_by(2).filter_map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok()).collect()));
            }
            '[' => out.push(Tok::Abre),
            ']' => out.push(Tok::Fecha),
            _ => {}
        }
    }
    out
}

fn codigo(b: &[u8]) -> u32 {
    b.iter().fold(0, |a, &x| (a << 8) | x as u32)
}

fn utf16(b: &[u8]) -> String {
    let w: Vec<u16> = b.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
    String::from_utf16_lossy(&w)
}

/// Lê seções bfchar/bfrange de um CMap ToUnicode. Devolve o mapa e o tamanho dos códigos em bytes.
fn ler_tounicode(dados: &[u8]) -> (HashMap<u32, String>, Option<usize>) {
    let s = String::from_utf8_lossy(dados);
    let mut mapa = HashMap::new();
    let mut bytes = None;
    for (ini, fim) in [("beginbfchar", "endbfchar"), ("beginbfrange", "endbfrange")] {
        let mut resto: &str = &s;
        while let Some(i) = resto.find(ini) {
            let corpo = &resto[i + ini.len()..];
            let Some(j) = corpo.find(fim) else { break };
            let toks = tokens(&corpo[..j]);
            resto = &corpo[j..];
            if ini == "beginbfchar" {
                for par in toks.chunks(2) {
                    if let [Tok::Hex(src), Tok::Hex(dst)] = par {
                        bytes.get_or_insert(src.len());
                        mapa.insert(codigo(src), utf16(dst));
                    }
                }
                continue;
            }
            let mut k = 0;
            while let (Some(Tok::Hex(lo)), Some(Tok::Hex(hi))) = (toks.get(k), toks.get(k + 1)) {
                bytes.get_or_insert(lo.len());
                let (lo_c, hi_c) = (codigo(lo), codigo(hi).min(codigo(lo) + 65_535));
                match toks.get(k + 2) {
                    Some(Tok::Hex(dst)) => {
                        let mut base: Vec<u16> = dst.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
                        for c in lo_c..=hi_c {
                            mapa.insert(c, String::from_utf16_lossy(&base));
                            if let Some(u) = base.last_mut() {
                                *u = u.wrapping_add(1);
                            }
                        }
                        k += 3;
                    }
                    Some(Tok::Abre) => {
                        let mut c = lo_c;
                        k += 3;
                        while let Some(Tok::Hex(dst)) = toks.get(k) {
                            mapa.insert(c, utf16(dst));
                            c += 1;
                            k += 1;
                        }
                        k += 1; // ']'
                    }
                    _ => break,
                }
            }
        }
    }
    (mapa, bytes)
}

// ───────────────────────────── Codificações ─────────────────────────────

const WIN_ANSI_80: &str = "€\0‚ƒ„…†‡ˆ‰Š‹Œ\0Ž\0\0‘’“”•–—˜™š›œ\0žŸ";

/// WinAnsiEncoding, ou StandardEncoding (fontes Type1 sem /Encoding) quando `padrao`
fn codigo_base(c: u8, padrao: bool) -> Option<String> {
    if padrao {
        const ALTO: [(u8, &str); 36] = [
            (0x27, "’"), (0x60, "‘"), (0xA1, "¡"), (0xA2, "¢"), (0xA3, "£"), (0xA4, "⁄"), (0xA5, "¥"), (0xA6, "ƒ"),
            (0xA7, "§"), (0xA9, "'"), (0xAA, "“"), (0xAB, "«"), (0xAC, "‹"), (0xAD, "›"), (0xAE, "fi"), (0xAF, "fl"),
            (0xB1, "–"), (0xB2, "†"), (0xB3, "‡"), (0xB4, "·"), (0xB6, "¶"), (0xB7, "•"), (0xB8, "‚"), (0xB9, "„"),
            (0xBA, "”"), (0xBB, "»"), (0xBC, "…"), (0xBD, "‰"), (0xBF, "¿"), (0xD0, "—"), (0xE1, "Æ"), (0xE8, "Ł"),
            (0xF1, "æ"), (0xF5, "ı"), (0xF8, "ł"), (0xFA, "œ"),
        ];
        if let Some((_, s)) = ALTO.iter().find(|(k, _)| *k == c) {
            return Some(s.to_string());
        }
        return (0x20..0x7F).contains(&c).then(|| (c as char).to_string());
    }
    match c {
        0x20..=0x7E | 0xA0..=0xFF => Some((c as char).to_string()),
        0x80..=0x9F => WIN_ANSI_80.chars().nth((c - 0x80) as usize).filter(|&ch| ch != '\0').map(String::from),
        _ => None,
    }
}

/// Nome de glifo (Adobe Glyph List, subconjunto comum em artigos) → Unicode
fn glifo(nome: &str) -> Option<String> {
    const ASCII: [&str; 95] = [
        "space", "exclam", "quotedbl", "numbersign", "dollar", "percent", "ampersand", "quotesingle", "parenleft",
        "parenright", "asterisk", "plus", "comma", "hyphen", "period", "slash", "zero", "one", "two", "three", "four",
        "five", "six", "seven", "eight", "nine", "colon", "semicolon", "less", "equal", "greater", "question", "at",
        "A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V",
        "W", "X", "Y", "Z", "bracketleft", "backslash", "bracketright", "asciicircum", "underscore", "grave", "a",
        "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r", "s", "t", "u", "v", "w",
        "x", "y", "z", "braceleft", "bar", "braceright", "asciitilde",
    ];
    const LATIN1: [&str; 64] = [
        "Agrave", "Aacute", "Acircumflex", "Atilde", "Adieresis", "Aring", "AE", "Ccedilla", "Egrave", "Eacute",
        "Ecircumflex", "Edieresis", "Igrave", "Iacute", "Icircumflex", "Idieresis", "Eth", "Ntilde", "Ograve",
        "Oacute", "Ocircumflex", "Otilde", "Odieresis", "multiply", "Oslash", "Ugrave", "Uacute", "Ucircumflex",
        "Udieresis", "Yacute", "Thorn", "germandbls", "agrave", "aacute", "acircumflex", "atilde", "adieresis",
        "aring", "ae", "ccedilla", "egrave", "eacute", "ecircumflex", "edieresis", "igrave", "iacute", "icircumflex",
        "idieresis", "eth", "ntilde", "ograve", "oacute", "ocircumflex", "otilde", "odieresis", "divide", "oslash",
        "ugrave", "uacute", "ucircumflex", "udieresis", "yacute", "thorn", "ydieresis",
    ];
    const GREGO: [&str; 25] = [
        "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa", "lambda", "mu", "nu",
        "xi", "omicron", "pi", "rho", "sigma1", "sigma", "tau", "upsilon", "phi", "chi", "psi", "omega",
    ];
    const OUTROS: [(&str, &str); 48] = [
        ("quoteright", "’"), ("quoteleft", "‘"), ("quotedblleft", "“"), ("quotedblright", "”"), ("quotesinglbase", "‚"),
        ("quotedblbase", "„"), ("endash", "–"), ("emdash", "—"), ("bullet", "•"), ("dagger", "†"), ("daggerdbl", "‡"),
        ("ellipsis", "…"), ("fi", "fi"), ("fl", "fl"), ("ff", "ff"), ("ffi", "ffi"), ("ffl", "ffl"), ("dotlessi", "ı"),
        ("minus", "−"), ("section", "§"), ("paragraph", "¶"), ("copyright", "©"), ("registered", "®"),
        ("trademark", "™"), ("degree", "°"), ("plusminus", "±"), ("periodcentered", "·"), ("lslash", "ł"),
        ("Lslash", "Ł"), ("oe", "œ"), ("OE", "Œ"), ("scaron", "š"), ("Scaron", "Š"), ("zcaron", "ž"), ("Zcaron", "Ž"),
        ("Euro", "€"), ("fraction", "⁄"), ("exclamdown", "¡"), ("questiondown", "¿"), ("guillemotleft", "«"),
        ("guillemotright", "»"), ("cent", "¢"), ("sterling", "£"), ("yen", "¥"), ("nbspace", " "), ("logicalnot", "¬"),
        ("acute", "´"), ("dieresis", "¨"),
    ];
    let nome = nome.split('.').next().unwrap_or(nome); // "a.sc", "one.oldstyle"
    if nome.contains('_') {
        // ligaduras nomeadas por componentes: "f_f_i"
        return nome.split('_').map(glifo).collect();
    }
    if let Some(hex) = nome.strip_prefix("uni").filter(|h| h.len() >= 4 && h.len() % 4 == 0) {
        let w: Option<Vec<u16>> = (0..hex.len()).step_by(4).map(|i| u16::from_str_radix(&hex[i..i + 4], 16).ok()).collect();
        return w.map(|w| String::from_utf16_lossy(&w));
    }
    if let Some(hex) = nome.strip_prefix('u').filter(|h| (4..=6).contains(&h.len()))
        && let Some(c) = u32::from_str_radix(hex, 16).ok().and_then(char::from_u32) {
            return Some(c.to_string());
        }
    let em = |tab: &[&str], base: u32| tab.iter().position(|&n| n == nome).and_then(|i| char::from_u32(base + i as u32)).map(String::from);
    em(&ASCII, 0x20)
        .or_else(|| em(&LATIN1, 0xC0))
        .or_else(|| em(&GREGO, 0x3B1))
        .or_else(|| OUTROS.iter().find(|(n, _)| *n == nome).map(|(_, s)| s.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deve_ler_cmap_tounicode() {
        let cmap = b"1 begincodespacerange <0000> <FFFF> endcodespacerange
            2 beginbfchar <0003> <0020> <0011> <00660069> endbfchar
            2 beginbfrange <0024> <0026> <0041> <0030> <0031> [<00E9> <00E7>] endbfrange";
        let (m, bytes) = ler_tounicode(cmap);
        assert_eq!(bytes, Some(2));
        assert_eq!(m[&0x0003], " ");
        assert_eq!(m[&0x0011], "fi");
        assert_eq!(m[&0x0026], "C");
        assert_eq!(m[&0x0031], "ç");
    }

    #[test]
    fn deve_mapear_nomes_de_glifos() {
        assert_eq!(glifo("eacute").as_deref(), Some("é"));
        assert_eq!(glifo("f_f_i").as_deref(), Some("ffi"));
        assert_eq!(glifo("uni00E7").as_deref(), Some("ç"));
        assert_eq!(glifo("a.sc").as_deref(), Some("a"));
        assert_eq!(glifo("quoteright").as_deref(), Some("’"));
        assert_eq!(glifo("glifoinexistente"), None);
    }

    #[test]
    fn deve_agrupar_trechos_em_linhas_com_sobrescrito_separado() {
        let t = |texto: &str, x: f64, y: f64, tam: f64| Trecho { texto: texto.into(), x, y, fim: x + texto.len() as f64 * tam * 0.5, tam, rotacionado: false };
        let mut linhas = Vec::new();
        montar_linhas(vec![
            t("Ana Silva", 100., 700., 12.),
            t("1", 154., 704., 7.),
            t(", John Doe", 158., 700., 12.),
            t("Abstract", 100., 680., 10.),
        ], 1, &mut linhas);
        assert_eq!(linhas.len(), 2);
        assert_eq!(linhas[0].texto, "Ana Silva 1, John Doe");
        assert_eq!(linhas[0].fonte, 12.);
        assert_eq!(linhas[1].texto, "Abstract");
    }

    #[test]
    fn deve_extrair_texto_de_pdf_gerado() {
        use lopdf::dictionary;
        let mut doc = Document::with_version("1.5");
        let font = doc.add_object(dictionary! { "Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica", "Encoding" => "WinAnsiEncoding" });
        let ops = vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 20.into()]),
            Operation::new("Td", vec![72.into(), 700.into()]),
            Operation::new("Tj", vec![Object::String(b"Um T\xedtulo".to_vec(), lopdf::StringFormat::Literal)]),
            Operation::new("Tf", vec!["F1".into(), 10.into()]),
            Operation::new("Td", vec![0.into(), (-30).into()]),
            Operation::new("TJ", vec![Object::Array(vec![Object::string_literal("Corpo"), (-300).into(), Object::string_literal("texto")])]),
            Operation::new("ET", vec![]),
        ];
        let conteudo = doc.add_object(lopdf::Stream::new(dictionary! {}, Content { operations: ops }.encode().unwrap()));
        let pages = doc.new_object_id();
        let page = doc.add_object(dictionary! { "Type" => "Page", "Parent" => pages, "Contents" => conteudo,
            "Resources" => dictionary! { "Font" => dictionary! { "F1" => font } } });
        doc.objects.insert(pages, Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => vec![page.into()], "Count" => 1 }));
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages });
        doc.trailer.set("Root", catalog);

        let linhas = extrair_linhas(&doc, [1]);
        assert_eq!(linhas.iter().map(|l| (l.texto.as_str(), l.fonte)).collect::<Vec<_>>(), vec![("Um Título", 20.), ("Corpo texto", 10.)]);
    }
}
