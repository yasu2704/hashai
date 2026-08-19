//! Conservative, lexical risk classification for generated shell commands.

use crate::runner::Risk;

/// The final risk is the monotonic maximum of model and local evidence.
pub fn combine(model: Risk, local: Risk) -> Risk {
    model.max(local)
}

/// Classify syntax that can be recognized without executing or parsing a shell.
pub fn analyze(command: &str) -> Risk {
    let (tokens, uncertain) = lex(command);
    let mut risk = if uncertain { Risk::Review } else { Risk::Safe };
    if command.contains('\n') || command.contains("<<") || command.contains("<<<") {
        risk = risk.max(Risk::Review);
    }
    if tokens.iter().any(|token| token == "|" || token == "||") {
        risk = risk.max(Risk::Review);
    }
    for (index, _) in tokens
        .iter()
        .enumerate()
        .filter(|(_, token)| token.as_str() == "|")
    {
        let before = segment_program(&tokens[..index]);
        let after = segment_program(&tokens[index + 1..]);
        if matches!(before, Some("curl" | "wget"))
            && matches!(after, Some("sh" | "bash" | "zsh" | "fish"))
        {
            risk = risk.max(Risk::Dangerous);
        }
    }
    if command.contains("$(") || command.contains('`') {
        risk = risk.max(Risk::Review);
    }
    if tokens
        .last()
        .is_some_and(|token| matches!(token.as_str(), ";" | "&&" | "||" | "|"))
    {
        risk = risk.max(Risk::Review);
    }

    let mut command_start = true;
    let mut current = Vec::new();
    for token in &tokens {
        if matches!(token.as_str(), ";" | "&&" | "||" | "|" | "&") {
            risk = risk.max(classify_words(&current));
            current.clear();
            command_start = true;
        } else {
            if command_start && matches!(token.as_str(), "rm" | "rmdir" | "sudo" | "su" | "dd") {
                risk = risk.max(Risk::Dangerous);
            }
            current.push(token.as_str());
            command_start = false;
        }
    }
    risk.max(classify_words(&current))
        .max(classify_redirects(command))
}

fn classify_words(words: &[&str]) -> Risk {
    if words.is_empty() {
        return Risk::Safe;
    }
    let offset = executable_offset(words);
    let Some(program) = words.get(offset).copied() else {
        return Risk::Review;
    };
    let words = &words[offset..];
    if program.starts_with("mkfs") || matches!(program, "fdisk" | "sfdisk" | "killall" | "pkill") {
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
    if program == "kill" && words.iter().any(|word| *word == "-1" || *word == "--all") {
        return Risk::Dangerous;
    }
    if program == "git" {
        if words.get(1) == Some(&"reset") && words.contains(&"--hard") {
            return Risk::Dangerous;
        }
        if words.get(1) == Some(&"clean")
            && !words.iter().any(|word| matches!(*word, "-n" | "--dry-run"))
        {
            return Risk::Dangerous;
        }
        if words.get(1) == Some(&"push")
            && words.iter().any(|word| matches!(*word, "--force" | "-f"))
        {
            return Risk::Dangerous;
        }
    }
    if matches!(program, "psql" | "mysql" | "sqlite3" | "sqlcmd")
        && words
            .iter()
            .flat_map(|word| word.split_whitespace())
            .any(|word| matches!(word, "DROP" | "drop" | "TRUNCATE" | "truncate"))
    {
        return Risk::Dangerous;
    }
    Risk::Safe
}

fn executable_offset(words: &[&str]) -> usize {
    let mut index = 0;
    while let Some(word) = words.get(index) {
        if word.contains('=') && !word.starts_with('=') {
            index += 1;
            continue;
        }
        if *word == "env" || *word == "command" {
            index += 1;
            continue;
        }
        break;
    }
    index
}

fn segment_program(tokens: &[String]) -> Option<&str> {
    let start = tokens
        .iter()
        .rposition(|word| matches!(word.as_str(), ";" | "&&" | "||" | "|" | "&"))
        .map_or(0, |index| index + 1);
    let words: Vec<&str> = tokens[start..].iter().map(String::as_str).collect();
    tokens
        .get(start + executable_offset(&words))
        .map(String::as_str)
}

fn classify_redirects(command: &str) -> Risk {
    let bytes = command.as_bytes();
    let mut quote = None;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if matches!(byte, b'\'' | b'"') {
            if quote == Some(byte) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(byte);
            }
            index += 1;
            continue;
        }
        if quote.is_none() && byte == b'<' {
            return Risk::Review;
        }
        if quote.is_none() && byte == b'>' {
            let rest = &command[index..];
            if rest.starts_with(">&") {
                index += 1;
                continue;
            }
            let target = rest[1..].trim_start();
            let target = target.strip_prefix('|').unwrap_or(target);
            let target = target
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .or_else(|| {
                    target
                        .strip_prefix('\'')
                        .and_then(|value| value.strip_suffix('\''))
                })
                .unwrap_or(target);
            if target == "/dev/null" {
                index += 1;
                continue;
            }
            if rest.starts_with(">>") {
                return Risk::Review;
            }
            if rest[1..].trim().is_empty() {
                return Risk::Review;
            }
            return Risk::Dangerous;
        }
        index += 1;
    }
    Risk::Safe
}

fn lex(input: &str) -> (Vec<String>, bool) {
    let mut tokens = Vec::new();
    let mut token = String::new();
    let mut quote = None;
    let mut uncertain = false;
    let chars: Vec<char> = input.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        if matches!(current, '\'' | '"') {
            if quote == Some(current) {
                quote = None;
            } else if quote.is_none() {
                quote = Some(current);
            } else {
                token.push(current);
            }
        } else if quote.is_none() && current == '#' && token.is_empty() {
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            if tokens.last().is_none_or(|token| token != ";") {
                tokens.push(";".to_owned());
            }
            continue;
        } else if quote.is_none() && current == '\n' {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
            tokens.push(";".to_owned());
            uncertain = true;
        } else if quote.is_none() && current.is_whitespace() {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
        } else if quote.is_none() && current == ';' {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
            tokens.push(";".to_owned());
        } else if quote.is_none() && current == '|' {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
            if chars.get(index + 1) == Some(&'|') {
                tokens.push("||".to_owned());
                index += 1;
            } else {
                tokens.push("|".to_owned());
            }
        } else if quote.is_none() && current == '&' && index > 0 && chars[index - 1] == '>' {
            token.push(current);
        } else if quote.is_none() && current == '&' && chars.get(index + 1) == Some(&'&') {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
            tokens.push("&&".to_owned());
            index += 1;
        } else if quote.is_none() && current == '&' {
            if !token.is_empty() {
                tokens.push(std::mem::take(&mut token));
            }
            tokens.push("&".to_owned());
            uncertain = true;
        } else if quote.is_none() && matches!(current, '(' | ')' | '{' | '}' | '\\') {
            uncertain = true;
            token.push(current);
        } else {
            token.push(current);
        }
        index += 1;
    }
    if !token.is_empty() {
        tokens.push(token);
    }
    if quote.is_some() {
        uncertain = true;
    }
    (tokens, uncertain)
}
