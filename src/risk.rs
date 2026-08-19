//! Conservative, lexical risk classification for generated shell commands.

use crate::runner::Risk;

/// The final risk is the monotonic maximum of model and local evidence.
pub fn combine(model: Risk, local: Risk) -> Risk {
    model.max(local)
}

/// Classify syntax that can be recognized without executing or parsing a shell.
pub fn analyze(command: &str) -> Risk {
    let scan = scan(command);
    let mut risk = if scan.uncertain {
        Risk::Review
    } else {
        Risk::Safe
    };
    if matches!(
        scan.lexemes.last(),
        Some(Lexeme::Operator(
            Operator::And | Operator::Or | Operator::Pipe | Operator::Background
        ))
    ) {
        risk = risk.max(Risk::Review);
    }
    let mut words = Vec::new();
    let mut pipe_source = None;
    for lexeme in &scan.lexemes {
        match lexeme {
            Lexeme::Word(word) => words.push(word.as_str()),
            Lexeme::Redirect(redirect) => risk = risk.max(redirect.risk()),
            Lexeme::Operator(operator) => {
                risk = risk.max(classify_words(&words));
                let program = executable(&words);
                risk = risk.max(remote_execution_risk(pipe_source, program));
                words.clear();
                if matches!(operator, Operator::Pipe | Operator::Or) {
                    risk = risk.max(Risk::Review);
                }
                pipe_source = matches!(operator, Operator::Pipe)
                    .then_some(program)
                    .flatten();
            }
        }
    }
    risk = risk.max(classify_words(&words));
    risk = risk.max(remote_execution_risk(pipe_source, executable(&words)));
    risk
}

fn remote_execution_risk(source: Option<&str>, target: Option<&str>) -> Risk {
    if matches!(source, Some("curl" | "wget"))
        && matches!(target, Some("sh" | "bash" | "zsh" | "fish"))
    {
        Risk::Dangerous
    } else {
        Risk::Safe
    }
}

#[derive(Debug)]
struct Scan {
    lexemes: Vec<Lexeme>,
    uncertain: bool,
}
#[derive(Debug)]
enum Lexeme {
    Word(String),
    Operator(Operator),
    Redirect(Redirect),
}
#[derive(Clone, Copy, Debug)]
enum Operator {
    Sequence,
    And,
    Or,
    Pipe,
    Background,
}
#[derive(Clone, Copy, Debug)]
enum RedirectKind {
    Truncate,
    Append,
    Input,
    ReadWrite,
    HereDoc,
    HereString,
    OutputDuplicate,
    InputDuplicate,
}
#[derive(Debug)]
struct Redirect {
    kind: RedirectKind,
    target: Option<String>,
}

impl Redirect {
    fn risk(&self) -> Risk {
        match self.kind {
            RedirectKind::Truncate => match self.target.as_deref() {
                Some("/dev/null") => Risk::Safe,
                Some(_) => Risk::Dangerous,
                None => Risk::Review,
            },
            RedirectKind::OutputDuplicate | RedirectKind::InputDuplicate => {
                if self.target.is_some() {
                    Risk::Safe
                } else {
                    Risk::Review
                }
            }
            RedirectKind::Append
            | RedirectKind::Input
            | RedirectKind::ReadWrite
            | RedirectKind::HereDoc
            | RedirectKind::HereString => Risk::Review,
        }
    }
}

/// A single lexical pass. Redirect words are attached here, so analysis never
/// rescans the original command text for redirection syntax.
fn scan(input: &str) -> Scan {
    let chars: Vec<char> = input.chars().collect();
    let mut lexemes = Vec::new();
    let mut word = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut uncertain = false;
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if escaped {
            word.push(current);
            escaped = false;
            index += 1;
            continue;
        }
        if current == '\\' {
            // Preserve the escaped character in the current word. In
            // particular, it must not be reconsidered as an operator,
            // comment marker, or quote delimiter on the next iteration.
            escaped = true;
            uncertain = true;
            index += 1;
            continue;
        }
        if let Some(open_quote) = quote {
            if current == open_quote {
                quote = None;
            } else {
                word.push(current);
            }
            index += 1;
            continue;
        }
        match current {
            '\'' | '"' => quote = Some(current),
            '#' if word.is_empty() => {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
                continue;
            }
            c if c.is_whitespace() => {
                push_word(&mut lexemes, &mut word);
                if c == '\n' {
                    lexemes.push(Lexeme::Operator(Operator::Sequence));
                    uncertain = true;
                }
            }
            ';' => {
                push_word(&mut lexemes, &mut word);
                lexemes.push(Lexeme::Operator(Operator::Sequence));
            }
            '|' => {
                push_word(&mut lexemes, &mut word);
                if chars.get(index + 1) == Some(&'|') {
                    lexemes.push(Lexeme::Operator(Operator::Or));
                    index += 1;
                } else {
                    lexemes.push(Lexeme::Operator(Operator::Pipe));
                }
            }
            '&' if chars.get(index + 1) == Some(&'>') => {
                push_word(&mut lexemes, &mut word);
                let (target, next) = redirect_target(&chars, index + 2);
                lexemes.push(Lexeme::Redirect(Redirect {
                    kind: RedirectKind::Truncate,
                    target,
                }));
                index = next - 1;
            }
            '&' if chars.get(index + 1) == Some(&'&') => {
                push_word(&mut lexemes, &mut word);
                lexemes.push(Lexeme::Operator(Operator::And));
                index += 1;
            }
            '&' => {
                push_word(&mut lexemes, &mut word);
                lexemes.push(Lexeme::Operator(Operator::Background));
                uncertain = true;
            }
            '<' | '>' => {
                let fd = word
                    .chars()
                    .all(|c| c.is_ascii_digit())
                    .then(|| std::mem::take(&mut word));
                if fd.is_none() {
                    push_word(&mut lexemes, &mut word);
                }
                let (kind, end) = redirect_kind(&chars, index, current);
                let (target, next) = redirect_target(&chars, end);
                lexemes.push(Lexeme::Redirect(Redirect { kind, target }));
                index = next - 1;
            }
            '$' if chars.get(index + 1) == Some(&'(') => {
                uncertain = true;
                word.push(current);
            }
            '`' | '(' | ')' | '{' | '}' => {
                uncertain = true;
                word.push(current);
            }
            _ => word.push(current),
        }
        index += 1;
    }
    push_word(&mut lexemes, &mut word);
    if quote.is_some() || escaped {
        uncertain = true;
    }
    Scan { lexemes, uncertain }
}

fn redirect_kind(chars: &[char], index: usize, current: char) -> (RedirectKind, usize) {
    let next = chars.get(index + 1).copied();
    let kind = match (current, next) {
        ('>', Some('>')) => RedirectKind::Append,
        ('>', Some('|')) => RedirectKind::Truncate,
        ('>', Some('&')) => RedirectKind::OutputDuplicate,
        ('<', Some('&')) => RedirectKind::InputDuplicate,
        ('<', Some('<')) if chars.get(index + 2) == Some(&'<') => RedirectKind::HereString,
        ('<', Some('<')) => RedirectKind::HereDoc,
        ('<', Some('>')) => RedirectKind::ReadWrite,
        ('<', _) => RedirectKind::Input,
        ('>', _) => RedirectKind::Truncate,
        _ => unreachable!(),
    };
    let width = match (current, next) {
        ('<', Some('<')) if chars.get(index + 2) == Some(&'<') => 3,
        ('>' | '<', Some('>') | Some('<') | Some('|') | Some('&')) => 2,
        _ => 1,
    };
    (kind, index + width)
}

fn redirect_target(chars: &[char], mut index: usize) -> (Option<String>, usize) {
    while chars
        .get(index)
        .is_some_and(|c| c.is_whitespace() && *c != '\n')
    {
        index += 1;
    }
    let Some(first) = chars.get(index).copied() else {
        return (None, index);
    };
    if matches!(first, '\n' | ';' | '|' | '&' | '<' | '>') {
        return (None, index);
    }
    let mut target = String::new();
    let mut quote = None;
    while let Some(current) = chars.get(index).copied() {
        if let Some(open_quote) = quote {
            if current == open_quote {
                quote = None;
            } else {
                target.push(current);
            }
            index += 1;
            continue;
        }
        if matches!(current, '\'' | '"') {
            quote = Some(current);
        } else if current.is_whitespace() || matches!(current, ';' | '|' | '&' | '<' | '>') {
            break;
        } else {
            target.push(current);
        }
        index += 1;
    }
    if quote.is_some() || target.is_empty() {
        (None, index)
    } else {
        (Some(target), index)
    }
}

fn push_word(lexemes: &mut Vec<Lexeme>, word: &mut String) {
    if !word.is_empty() {
        lexemes.push(Lexeme::Word(std::mem::take(word)));
    }
}

fn classify_words(words: &[&str]) -> Risk {
    let Some(program) = executable(words) else {
        return if words.is_empty() {
            Risk::Safe
        } else {
            Risk::Review
        };
    };
    if program.starts_with("mkfs")
        || matches!(
            program,
            "fdisk"
                | "sfdisk"
                | "cfdisk"
                | "killall"
                | "pkill"
                | "rm"
                | "rmdir"
                | "sudo"
                | "su"
                | "dd"
        )
    {
        return Risk::Dangerous;
    }
    if program == "parted"
        && words
            .iter()
            .any(|word| matches!(*word, "rm" | "mklabel" | "mkpart"))
    {
        return Risk::Dangerous;
    }
    if program == "diskutil"
        && words
            .iter()
            .any(|word| matches!(*word, "eraseDisk" | "partitionDisk" | "eraseVolume"))
    {
        return Risk::Dangerous;
    }
    if matches!(program, "chmod" | "chown")
        && words
            .iter()
            .any(|word| matches!(*word, "-R" | "--recursive"))
    {
        return Risk::Dangerous;
    }
    if program == "kill" && words.iter().any(|word| matches!(*word, "-1" | "--all")) {
        return Risk::Dangerous;
    }
    let program_at = executable_index(words);
    if program == "git" {
        if words.get(program_at + 1) == Some(&"reset") && words.contains(&"--hard") {
            return Risk::Dangerous;
        }
        if words.get(program_at + 1) == Some(&"clean")
            && !words.iter().any(|word| matches!(*word, "-n" | "--dry-run"))
        {
            return Risk::Dangerous;
        }
        if words.get(program_at + 1) == Some(&"push")
            && words.iter().any(|word| matches!(*word, "--force" | "-f"))
        {
            return Risk::Dangerous;
        }
    }
    if matches!(program, "psql" | "mysql" | "sqlite3" | "sqlcmd")
        && words.iter().any(|word| sql_contains_destructive(word))
    {
        return Risk::Dangerous;
    }
    Risk::Safe
}

fn sql_contains_destructive(sql: &str) -> bool {
    let bytes = sql.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"--") {
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            index += 2;
            while index + 1 < bytes.len() && !bytes[index..].starts_with(b"*/") {
                index += 1;
            }
            index = (index + 2).min(bytes.len());
            continue;
        }
        if bytes[index] == b'\'' {
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\'' {
                    if bytes.get(index + 1) == Some(&b'\'') {
                        index += 2;
                    } else {
                        index += 1;
                        break;
                    }
                } else {
                    index += 1;
                }
            }
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_alphabetic() {
            index += 1;
        }
        if index > start
            && (bytes[start..index].eq_ignore_ascii_case(b"drop")
                || bytes[start..index].eq_ignore_ascii_case(b"truncate"))
        {
            return true;
        }
        if index == start {
            index += 1;
        }
    }
    false
}

fn executable<'a>(words: &[&'a str]) -> Option<&'a str> {
    words.get(executable_index(words)).copied()
}
fn executable_index(words: &[&str]) -> usize {
    let mut index = 0;
    while let Some(word) = words.get(index) {
        if is_assignment(word) {
            index += 1;
            continue;
        }
        if *word == "env" {
            index += 1;
            let mut options = true;
            while let Some(argument) = words.get(index) {
                if options && *argument == "--" {
                    options = false;
                    index += 1;
                    continue;
                }
                if options && matches!(*argument, "-u" | "--unset") {
                    index += 2;
                    continue;
                }
                if options && argument.starts_with('-') {
                    index += 1;
                    continue;
                }
                if is_assignment(argument) {
                    index += 1;
                    continue;
                }
                break;
            }
            continue;
        }
        if *word == "command" {
            index += 1;
            while let Some(argument) = words.get(index) {
                if *argument == "--" {
                    index += 1;
                    break;
                }
                if argument.starts_with('-') {
                    index += 1;
                } else {
                    break;
                }
            }
            continue;
        }
        break;
    }
    index
}
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    !name.is_empty()
        && name.chars().enumerate().all(|(i, c)| {
            if i == 0 {
                c == '_' || c.is_ascii_alphabetic()
            } else {
                c == '_' || c.is_ascii_alphanumeric()
            }
        })
}
