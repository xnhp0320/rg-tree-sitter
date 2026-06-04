use rg_tree_sitter_core::{
    filter_definitions_from_input, find_definitions, find_references, guess_language_from_path,
    LanguageId,
};
use std::io::{self, Read};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum OutputFormat {
    Plain,
    Json,
}

/// Shared CLI arguments.
#[derive(clap::Args, Debug)]
pub struct SearchArgs {
    /// Symbol name to search
    pub symbol: String,

    /// Programming language
    #[arg(long, short)]
    pub lang: Option<String>,

    /// Search directory
    #[arg(long, short, default_value = ".")]
    pub dir: PathBuf,

    /// Output format
    #[arg(long, value_enum, default_value = "plain")]
    pub format: OutputFormat,

    /// Connect to daemon via Unix socket
    #[arg(long)]
    pub socket: Option<PathBuf>,
}

pub fn run_define(args: &SearchArgs) -> anyhow::Result<()> {
    if let Some(socket) = &args.socket {
        match query_daemon(socket, "define", &args.symbol, &args.dir, args.lang.as_deref()) {
            Ok(matches) => {
                print_matches(&matches, args.format);
                return Ok(());
            }
            Err(e) => {
                eprintln!("Warning: daemon unavailable ({}), falling back to local search", e);
            }
        }
    }
    let lang = resolve_lang(args.lang.as_deref(), &args.dir)?;
    let matches = find_definitions(&args.symbol, &args.dir, lang)?;
    print_matches(&matches, args.format);
    Ok(())
}

pub fn run_refs(args: &SearchArgs) -> anyhow::Result<()> {
    if let Some(socket) = &args.socket {
        match query_daemon(socket, "refs", &args.symbol, &args.dir, args.lang.as_deref()) {
            Ok(matches) => {
                print_matches(&matches, args.format);
                return Ok(());
            }
            Err(e) => {
                eprintln!("Warning: daemon unavailable ({}), falling back to local search", e);
            }
        }
    }
    let lang = resolve_lang(args.lang.as_deref(), &args.dir)?;
    let matches = find_references(&args.symbol, &args.dir, lang)?;
    print_matches(&matches, args.format);
    Ok(())
}

pub fn run_filter(lang_str: &str, format: OutputFormat, socket: Option<&PathBuf>) -> anyhow::Result<()> {
    let lang = LanguageId::from_str(lang_str)
        .ok_or_else(|| anyhow::anyhow!("unsupported language: {}", lang_str))?;

    let mut input = String::new();
    io::stdin().read_to_string(&mut input)?;

    if let Some(socket) = socket {
        match filter_via_daemon(socket, lang, input.as_bytes()) {
            Ok(matches) => {
                print_matches(&matches, format);
                return Ok(());
            }
            Err(e) => {
                eprintln!("Warning: daemon unavailable ({}), falling back to local filter", e);
            }
        }
    }

    let matches = filter_definitions_from_input(input.as_bytes(), lang)?;
    print_matches(&matches, format);
    Ok(())
}

fn resolve_lang(lang: Option<&str>, dir: &std::path::Path) -> anyhow::Result<LanguageId> {
    if let Some(l) = lang {
        LanguageId::from_str(l)
            .ok_or_else(|| anyhow::anyhow!("unsupported language: {}", l))
    } else {
        // Try to guess from files in dir
        if let Some(l) = guess_from_dir(dir) {
            Ok(l)
        } else {
            Err(anyhow::anyhow!(
                "could not guess language; please specify --lang"
            ))
        }
    }
}

fn guess_from_dir(dir: &std::path::Path) -> Option<LanguageId> {
    // Simple heuristic: count file extensions
    let mut counts = std::collections::HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            if let Some(lang) = guess_language_from_path(&entry.path()) {
                *counts.entry(lang).or_insert(0) += 1;
            }
        }
    }
    counts.into_iter().max_by_key(|(_, c)| *c).map(|(l, _)| l)
}

fn print_matches(matches: &[rg_tree_sitter_core::SemanticMatch], format: OutputFormat) {
    match format {
        OutputFormat::Plain => {
            for m in matches {
                println!("{}:{}:{}:{}", m.path, m.line, m.column, m.text);
            }
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(matches).unwrap());
        }
    }
}

// --- IPC client helpers ---

fn query_daemon(
    socket: &std::path::Path,
    command: &str,
    symbol: &str,
    dir: &std::path::Path,
    lang: Option<&str>,
) -> anyhow::Result<Vec<rg_tree_sitter_core::SemanticMatch>> {
    use std::io::{BufRead, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket)?;
    let req = crate::daemon::QueryRequest {
        command: command.to_string(),
        symbol: symbol.to_string(),
        lang: lang.and_then(LanguageId::from_str).unwrap_or(LanguageId::Cpp),
        dir: dir.to_path_buf(),
    };
    let req_json = serde_json::to_string(&req)?;
    stream.write_all(req_json.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let reader = std::io::BufReader::new(&stream);
    let resp_line = reader.lines().next().transpose()?.unwrap_or_default();
    let resp: crate::daemon::QueryResponse = serde_json::from_str(&resp_line)?;
    Ok(resp.matches)
}

fn filter_via_daemon(
    socket: &std::path::Path,
    lang: LanguageId,
    input: &[u8],
) -> anyhow::Result<Vec<rg_tree_sitter_core::SemanticMatch>> {
    use std::io::{BufRead, Write};
    use std::os::unix::net::UnixStream;

    let mut stream = UnixStream::connect(socket)?;
    // For filter we reuse the same request format but encode stdin data in a simple way:
    // send a special command with the raw matches.
    let text = String::from_utf8_lossy(input);
    let req = crate::daemon::QueryRequest {
        command: "filter".to_string(),
        symbol: text.to_string(),
        lang,
        dir: std::path::PathBuf::from("."),
    };
    let req_json = serde_json::to_string(&req)?;
    stream.write_all(req_json.as_bytes())?;
    stream.write_all(b"\n")?;
    stream.shutdown(std::net::Shutdown::Write)?;

    let reader = std::io::BufReader::new(&stream);
    let resp_line = reader.lines().next().transpose()?.unwrap_or_default();
    let resp: crate::daemon::QueryResponse = serde_json::from_str(&resp_line)?;
    Ok(resp.matches)
}
