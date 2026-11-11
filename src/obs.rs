//! Obsidian-flavoured helpers that need no GTK: callout detection, vault
//! lookup, LaTeX -> Unicode, tiny syntax highlighter, YAML properties.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ------------------------------------------------------------ callouts

pub struct CalloutKind {
    pub name: &'static str,
    pub rgb: (f32, f32, f32),
    pub icon: &'static str,
}

pub fn callout_kind(k: &str) -> CalloutKind {
    let blue = (0.03, 0.43, 0.87);
    let cyan = (0.0, 0.75, 0.74);
    let green = (0.03, 0.73, 0.31);
    let orange = (0.93, 0.46, 0.0);
    let red = (0.91, 0.19, 0.28);
    let purple = (0.47, 0.32, 0.93);
    let grey = (0.62, 0.62, 0.62);
    let (name, rgb, icon) = match k {
        "note" => ("note", blue, "\u{270E}"),
        "abstract" | "summary" | "tldr" => ("abstract", cyan, "\u{2261}"),
        "info" => ("info", blue, "\u{2139}"),
        "todo" => ("todo", blue, "\u{2611}"),
        "tip" | "hint" | "important" => ("tip", cyan, "\u{2726}"),
        "success" | "check" | "done" => ("success", green, "\u{2714}"),
        "question" | "help" | "faq" => ("question", orange, "?"),
        "warning" | "caution" | "attention" => ("warning", orange, "\u{26A0}"),
        "failure" | "fail" | "missing" => ("failure", red, "\u{2718}"),
        "danger" | "error" => ("danger", red, "\u{26A1}"),
        "bug" => ("bug", red, "\u{2731}"),
        "example" => ("example", purple, "\u{2263}"),
        "quote" | "cite" => ("quote", grey, "\u{275D}"),
        _ => ("note", blue, "\u{270E}"),
    };
    CalloutKind { name, rgb, icon }
}

#[derive(Clone)]
pub struct CalloutInfo {
    pub kind: String,
    /// None = not foldable, Some(true) = open by default, Some(false) = collapsed
    pub fold: Option<bool>,
    pub title: String,
}

/// Remove `[!type]±` markers (line numbers stay valid) and return the callouts keyed by (line, quote depth).
pub fn prep_callouts(md: &str) -> (String, HashMap<(usize, usize), CalloutInfo>) {
    let mut map = HashMap::new();
    if !md.contains("[!") {
        return (md.to_string(), map);
    }
    let mut out = String::with_capacity(md.len());
    let mut fence: Option<char> = None;
    for (n, line) in md.split_inclusive('\n').enumerate() {
        let t = line.trim_start();
        if t.starts_with("```") || t.starts_with("~~~") {
            let c = t.chars().next().unwrap();
            match fence {
                None => fence = Some(c),
                Some(f) if f == c => fence = None,
                _ => {}
            }
        }
        if fence.is_none() {
            if let Some((depth, a, b, info)) = scan_callout(line) {
                map.insert((n, depth), info);
                out.push_str(&line[..a]);
                out.push_str(line[b..].trim_start_matches(' '));
                continue;
            }
        }
        out.push_str(line);
    }
    (out, map)
}

fn scan_callout(line: &str) -> Option<(usize, usize, usize, CalloutInfo)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut depth = 0;
    loop {
        let mut j = i;
        while j < bytes.len() && bytes[j] == b' ' && j - i < 4 {
            j += 1;
        }
        if j < bytes.len() && bytes[j] == b'>' {
            depth += 1;
            i = j + 1;
            if i < bytes.len() && bytes[i] == b' ' {
                i += 1;
            }
        } else {
            break;
        }
    }
    if depth == 0 || !line[i..].starts_with("[!") {
        return None;
    }
    let rest = &line[i + 2..];
    let close = rest.find(']')?;
    let kind = &rest[..close];
    if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return None;
    }
    let mut end = i + 2 + close + 1;
    let fold = match line[end..].chars().next() {
        Some('+') => {
            end += 1;
            Some(true)
        }
        Some('-') => {
            end += 1;
            Some(false)
        }
        _ => None,
    };
    let title = line[end..].trim().to_string();
    Some((depth, i, end, CalloutInfo { kind: kind.to_lowercase(), fold, title }))
}

pub fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

// ------------------------------------------------------------ vault

pub fn vault_root(base: &Path) -> PathBuf {
    base.ancestors().take(12).find(|a| a.join(".obsidian").is_dir()).unwrap_or(base).to_path_buf()
}

pub struct Vault {
    root: PathBuf,
    base: PathBuf,
    files: Option<HashMap<String, PathBuf>>,
}

impl Vault {
    pub fn new(base: &Path) -> Vault {
        Vault { root: vault_root(base), base: base.to_path_buf(), files: None }
    }

    fn index(&mut self) -> &HashMap<String, PathBuf> {
        if self.files.is_none() {
            let mut m: HashMap<String, PathBuf> = HashMap::new();
            let mut queue = std::collections::VecDeque::new();
            queue.push_back((self.root.clone(), 0));
            let mut count = 0;
            while let Some((dir, depth)) = queue.pop_front() {
                let Ok(rd) = std::fs::read_dir(&dir) else { continue };
                for e in rd.flatten() {
                    let name = e.file_name().to_string_lossy().into_owned();
                    if name.starts_with('.') {
                        continue;
                    }
                    let Ok(ft) = e.file_type() else { continue };
                    if ft.is_dir() {
                        if depth < 8 && name != "node_modules" && name != "target" {
                            queue.push_back((e.path(), depth + 1));
                        }
                    } else {
                        count += 1;
                        if count > 40000 {
                            break;
                        }
                        let low = name.to_lowercase();
                        m.entry(low).or_insert_with(|| e.path());
                    }
                }
            }
            self.files = Some(m);
        }
        self.files.as_ref().unwrap()
    }

    /// Resolve a wikilink target (no `#fragment`) to a file.
    pub fn find(&mut self, name: &str) -> Option<PathBuf> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        for root in [&self.base, &self.root] {
            for cand in [root.join(name), root.join(format!("{name}.md"))] {
                if cand.is_file() {
                    return Some(cand);
                }
            }
        }
        let leaf = name.rsplit('/').next().unwrap_or(name).to_lowercase();
        let idx = self.index();
        idx.get(&leaf).or_else(|| idx.get(&format!("{leaf}.md"))).cloned()
    }
}

pub fn resolve_note(base: &Path, name: &str) -> Option<PathBuf> {
    Vault::new(base).find(name)
}

pub fn slug(s: &str) -> String {
    s.trim().to_lowercase().chars().filter(|c| c.is_alphanumeric() || *c == ' ' || *c == '-').collect::<String>().replace(' ', "-")
}

/// Cut the part of a note an embed `![[Note#Heading]]` / `![[Note#^id]]` points at.
pub fn section_of(text: &str, frag: &str) -> String {
    let text = strip_frontmatter(text);
    if frag.is_empty() {
        return text.to_string();
    }
    let lines: Vec<&str> = text.lines().collect();
    if let Some(id) = frag.strip_prefix('^') {
        let needle = format!("^{id}");
        if let Some(i) = lines.iter().position(|l| l.trim_end().ends_with(&needle)) {
            let mut a = i;
            while a > 0 && !lines[a - 1].trim().is_empty() {
                a -= 1;
            }
            let mut b = i;
            while b + 1 < lines.len() && !lines[b + 1].trim().is_empty() {
                b += 1;
            }
            return lines[a..=b].join("\n");
        }
        return String::new();
    }
    let want = slug(frag);
    let level = |l: &str| l.chars().take_while(|c| *c == '#').count();
    let mut start = None;
    let mut lvl = 0;
    let mut fence = false;
    for (i, l) in lines.iter().enumerate() {
        if l.trim_start().starts_with("```") {
            fence = !fence;
        }
        if fence {
            continue;
        }
        let n = level(l);
        if (1..=6).contains(&n) && l[n..].starts_with(' ') {
            match start {
                None if slug(&l[n..]) == want => {
                    start = Some(i);
                    lvl = n;
                }
                Some(s) if n <= lvl => return lines[s..i].join("\n"),
                _ => {}
            }
        }
    }
    start.map(|s| lines[s..].join("\n")).unwrap_or_default()
}

pub fn strip_frontmatter(t: &str) -> &str {
    if let Some(rest) = t.strip_prefix("---\n") {
        if let Some(i) = rest.find("\n---") {
            let after = &rest[i + 4..];
            return after.trim_start_matches(['\r', '\n']);
        }
    }
    t
}

// ------------------------------------------------------------ properties

pub fn parse_props(yaml: &str) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = vec![];
    for l in yaml.lines() {
        if l.trim().is_empty() || l.trim_start().starts_with('#') {
            continue;
        }
        let t = l.trim_start();
        if let Some(item) = t.strip_prefix("- ") {
            if let Some(last) = rows.last_mut() {
                if !last.1.is_empty() {
                    last.1.push_str(", ");
                }
                last.1.push_str(clean_val(item).as_str());
            }
        } else if let Some((k, v)) = l.split_once(':') {
            if l.starts_with(' ') {
                continue;
            }
            rows.push((k.trim().to_string(), clean_val(v)));
        }
    }
    rows
}

fn clean_val(v: &str) -> String {
    let v = v.trim().trim_start_matches('[').trim_end_matches(']');
    v.split(',').map(|x| x.trim().trim_matches(['"', '\'']).to_string()).collect::<Vec<_>>().join(", ")
}

// ------------------------------------------------------------ LaTeX -> Unicode

fn sym(name: &str) -> Option<&'static str> {
    Some(match name {
        "alpha" => "α", "beta" => "β", "gamma" => "γ", "delta" => "δ", "epsilon" | "varepsilon" => "ε",
        "zeta" => "ζ", "eta" => "η", "theta" | "vartheta" => "θ", "iota" => "ι", "kappa" => "κ",
        "lambda" => "λ", "mu" => "μ", "nu" => "ν", "xi" => "ξ", "pi" => "π", "rho" => "ρ",
        "sigma" => "σ", "tau" => "τ", "upsilon" => "υ", "phi" | "varphi" => "φ", "chi" => "χ",
        "psi" => "ψ", "omega" => "ω", "Gamma" => "Γ", "Delta" => "Δ", "Theta" => "Θ", "Lambda" => "Λ",
        "Xi" => "Ξ", "Pi" => "Π", "Sigma" => "Σ", "Phi" => "Φ", "Psi" => "Ψ", "Omega" => "Ω",
        "cdot" => "·", "times" => "×", "div" => "÷", "pm" => "±", "mp" => "∓", "leq" | "le" => "≤",
        "geq" | "ge" => "≥", "neq" | "ne" => "≠", "approx" => "≈", "equiv" => "≡", "sim" => "∼",
        "infty" => "∞", "partial" => "∂", "nabla" => "∇", "sum" => "∑", "prod" => "∏", "int" => "∫",
        "oint" => "∮", "sqrt" => "√", "in" => "∈", "notin" => "∉", "subset" => "⊂", "supset" => "⊃",
        "subseteq" => "⊆", "cup" => "∪", "cap" => "∩", "emptyset" => "∅", "forall" => "∀",
        "exists" => "∃", "neg" => "¬", "land" | "wedge" => "∧", "lor" | "vee" => "∨",
        "to" | "rightarrow" => "→", "leftarrow" => "←", "leftrightarrow" => "↔", "Rightarrow" => "⇒",
        "Leftarrow" => "⇐", "Leftrightarrow" | "iff" => "⇔", "mapsto" => "↦", "ldots" | "dots" | "cdots" => "…",
        "degree" => "°", "angle" => "∠", "perp" => "⊥", "parallel" => "∥", "propto" => "∝",
        "hbar" => "ℏ", "ell" => "ℓ", "mathbb" | "mathbf" | "mathrm" | "mathcal" | "text" | "mathit" | "operatorname" => "",
        "left" | "right" | "big" | "Big" | "bigg" => "", "quad" | "qquad" => "  ", "," | ";" | ":" | "!" => " ",
        "lbrace" => "{", "rbrace" => "}", "langle" => "⟨", "rangle" => "⟩", "prime" => "′",
        "lim" => "lim", "sin" => "sin", "cos" => "cos", "tan" => "tan", "log" => "log", "ln" => "ln", "exp" => "exp",
        "min" => "min", "max" => "max", "det" => "det", "dim" => "dim",
        _ => return None,
    })
}

fn sup_char(c: char) -> Option<char> {
    Some(match c {
        '0' => '⁰', '1' => '¹', '2' => '²', '3' => '³', '4' => '⁴', '5' => '⁵', '6' => '⁶', '7' => '⁷',
        '8' => '⁸', '9' => '⁹', '+' => '⁺', '-' => '⁻', '=' => '⁼', '(' => '⁽', ')' => '⁾', 'n' => 'ⁿ', 'i' => 'ⁱ',
        _ => return None,
    })
}

fn sub_char(c: char) -> Option<char> {
    Some(match c {
        '0' => '₀', '1' => '₁', '2' => '₂', '3' => '₃', '4' => '₄', '5' => '₅', '6' => '₆', '7' => '₇',
        '8' => '₈', '9' => '₉', '+' => '₊', '-' => '₋', '=' => '₌', '(' => '₍', ')' => '₎',
        'a' => 'ₐ', 'e' => 'ₑ', 'o' => 'ₒ', 'x' => 'ₓ', 'i' => 'ᵢ', 'j' => 'ⱼ', 'n' => 'ₙ',
        _ => return None,
    })
}

pub fn latex(src: &str) -> String {
    let c: Vec<char> = src.chars().collect();
    let mut i = 0;
    conv(&c, &mut i, false)
}

fn group(c: &[char], i: &mut usize) -> String {
    // parse one argument: {...} or a single token
    while *i < c.len() && c[*i] == ' ' {
        *i += 1;
    }
    if *i >= c.len() {
        return String::new();
    }
    if c[*i] == '{' {
        *i += 1;
        let s = conv(c, i, true);
        s
    } else if c[*i] == '\\' {
        let mut j = *i + 1;
        while j < c.len() && c[j].is_ascii_alphabetic() {
            j += 1;
        }
        let end = j.max(*i + 2).min(c.len());
        let mut k = *i;
        let s = conv(&c[..end], &mut k, false);
        *i = end;
        s
    } else {
        let s = c[*i].to_string();
        *i += 1;
        s
    }
}

fn conv(c: &[char], i: &mut usize, until_brace: bool) -> String {
    let mut out = String::new();
    while *i < c.len() {
        let ch = c[*i];
        match ch {
            '}' if until_brace => {
                *i += 1;
                return out;
            }
            '{' => {
                *i += 1;
                out.push_str(&conv(c, i, true));
            }
            '\\' => {
                *i += 1;
                let st = *i;
                while *i < c.len() && c[*i].is_ascii_alphabetic() {
                    *i += 1;
                }
                let name: String = if *i == st && *i < c.len() {
                    *i += 1;
                    c[st..*i].iter().collect()
                } else {
                    c[st..*i].iter().collect()
                };
                match name.as_str() {
                    "frac" | "dfrac" | "tfrac" => {
                        let a = group(c, i);
                        let b = group(c, i);
                        let wrap = |s: String| if s.chars().count() > 1 && !s.chars().all(|x| x.is_alphanumeric()) { format!("({s})") } else { s };
                        out.push_str(&format!("{}⁄{}", wrap(a), wrap(b)));
                    }
                    "sqrt" => {
                        let a = group(c, i);
                        if a.chars().count() > 1 { out.push_str(&format!("√({a})")) } else { out.push_str(&format!("√{a}")) }
                    }
                    "\\" => out.push('\n'),
                    "begin" | "end" => {
                        let _ = group(c, i);
                    }
                    n => match sym(n) {
                        Some(s) => out.push_str(s),
                        None => out.push_str(n),
                    },
                }
            }
            '^' | '_' => {
                *i += 1;
                let g = group(c, i);
                let f = if ch == '^' { sup_char } else { sub_char };
                let mapped: Option<String> = g.chars().map(f).collect();
                match mapped {
                    Some(m) => out.push_str(&m),
                    None => out.push_str(&format!("{}{}", if ch == '^' { "^" } else { "_" }, if g.chars().count() > 1 { format!("({g})") } else { g })),
                }
            }
            '&' => {
                out.push_str("  ");
                *i += 1;
            }
            _ => {
                out.push(ch);
                *i += 1;
            }
        }
    }
    out
}

// ------------------------------------------------------------ syntax highlighting

#[derive(Clone, Copy, PartialEq)]
pub enum Hl {
    Kw,
    Str,
    Com,
    Num,
}

const KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "def", "default", "del", "do",
    "elif", "else", "enum", "except", "export", "extends", "extern", "false", "final", "finally", "fn", "for", "from",
    "func", "function", "global", "go", "if", "impl", "import", "in", "interface", "is", "lambda", "let", "loop", "match",
    "mod", "mut", "namespace", "new", "nil", "none", "None", "not", "null", "or", "and", "package", "pass", "priv", "private",
    "protected", "pub", "public", "raise", "return", "self", "static", "struct", "super", "switch", "this", "throw",
    "throws", "trait", "true", "True", "False", "try", "type", "typeof", "union", "unsafe", "use", "using", "var", "void",
    "where", "while", "with", "yield", "fi", "then", "done", "esac", "echo", "select", "insert", "update", "delete",
    "create", "table", "values", "set", "join", "on", "group", "order", "by", "limit", "SELECT", "FROM", "WHERE",
    "INSERT", "INTO", "UPDATE", "DELETE", "CREATE", "TABLE", "VALUES", "JOIN", "ON", "GROUP", "ORDER", "BY", "AND", "OR",
    "NOT", "NULL", "AS", "LIMIT", "int", "float", "double", "bool", "char", "string", "str", "i32", "i64", "u8", "u32",
    "u64", "usize", "String", "Vec", "Option", "Some", "Ok", "Err", "boolean", "number",
];

pub fn highlight(lang: &str, code: &str) -> Vec<(usize, usize, Hl)> {
    let lang = lang.to_lowercase();
    if matches!(lang.as_str(), "text" | "txt" | "plain" | "output" | "log" | "mermaid" | "console") || code.len() > 120_000 {
        return vec![];
    }
    let hash_c = matches!(lang.as_str(), "py" | "python" | "sh" | "bash" | "zsh" | "shell" | "yaml" | "yml" | "toml" | "ruby" | "rb" | "perl" | "make" | "makefile" | "dockerfile" | "r" | "conf" | "ini" | "nix" | "powershell");
    let dash_c = matches!(lang.as_str(), "sql" | "lua" | "haskell" | "hs");
    let slash_c = !hash_c && !dash_c && !matches!(lang.as_str(), "json" | "html" | "xml" | "css" | "md" | "markdown");
    let css_c = matches!(lang.as_str(), "css" | "scss" | "less" | "json" | "jsonc");
    let html_c = matches!(lang.as_str(), "html" | "xml" | "svg");
    let ch_lit = matches!(lang.as_str(), "rust" | "rs" | "c" | "cpp" | "c++" | "go" | "java" | "cs" | "csharp" | "kotlin" | "swift" | "");
    let ch: Vec<char> = code.chars().collect();
    let n = ch.len();
    let mut out = vec![];
    let mut i = 0;
    let starts = |i: usize, s: &str| s.chars().enumerate().all(|(k, c)| ch.get(i + k) == Some(&c));
    while i < n {
        let c = ch[i];
        if slash_c && starts(i, "//") || hash_c && c == '#' || dash_c && starts(i, "--") {
            let st = i;
            while i < n && ch[i] != '\n' {
                i += 1;
            }
            out.push((st, i, Hl::Com));
        } else if (slash_c || css_c) && starts(i, "/*") || html_c && starts(i, "<!--") {
            let st = i;
            let end = if html_c { "-->" } else { "*/" };
            i += 2;
            while i < n && !starts(i, end) {
                i += 1;
            }
            i = (i + end.len()).min(n);
            out.push((st, i, Hl::Com));
        } else if c == '"' || c == '`' || c == '\'' {
            let st = i;
            i += 1;
            if c == '\'' && ch_lit {
                // char literal only
                let mut j = i;
                if ch.get(j) == Some(&'\\') {
                    j += 1;
                }
                j += 1;
                if ch.get(j) == Some(&'\'') {
                    out.push((st, j + 1, Hl::Str));
                    i = j + 1;
                }
                continue;
            }
            let triple = (c == '"' || c == '\'') && starts(st, &c.to_string().repeat(3)) && matches!(lang.as_str(), "py" | "python");
            if triple {
                i = st + 3;
                while i < n && !starts(i, &c.to_string().repeat(3)) {
                    i += 1;
                }
                i = (i + 3).min(n);
                out.push((st, i, Hl::Str));
                continue;
            }
            while i < n && ch[i] != c && (ch[i] != '\n' || c == '`') {
                if ch[i] == '\\' {
                    i += 1;
                }
                i += 1;
            }
            let e = if i < n && ch[i] == c { i + 1 } else { i.min(n) };
            // an unterminated ' is just an apostrophe
            if c == '\'' && !(e > st + 1 && ch.get(e - 1) == Some(&'\'')) {
                i = st + 1;
                continue;
            }
            i = e;
            out.push((st, i, Hl::Str));
        } else if c.is_ascii_digit() && (i == 0 || !(ch[i - 1].is_alphanumeric() || ch[i - 1] == '_')) {
            let st = i;
            while i < n && (ch[i].is_ascii_alphanumeric() || ch[i] == '.' || ch[i] == '_') {
                i += 1;
            }
            out.push((st, i, Hl::Num));
        } else if c.is_alphabetic() || c == '_' {
            let st = i;
            while i < n && (ch[i].is_alphanumeric() || ch[i] == '_') {
                i += 1;
            }
            let w: String = ch[st..i].iter().collect();
            if KEYWORDS.contains(&w.as_str()) {
                out.push((st, i, Hl::Kw));
            }
        } else {
            i += 1;
        }
    }
    out
}
