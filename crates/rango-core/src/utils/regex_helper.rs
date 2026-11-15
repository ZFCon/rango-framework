//! `utils.regex_helper.normalize`: turn a regular expression into the
//! `(format_string, parameter_names)` forms used for reverse URL resolving.
//!
//! Not a regex decompiler: repeated sections keep their minimum count, optional groups with
//! parameters yield both forms, character classes use their first member, `|` is unsupported.

#[derive(Clone, Debug)]
enum Item {
    Ch(char),
    /// `(format, parameter name)`: a capturing group.
    Group(String, Option<String>),
    NonCapture(Vec<Item>),
    /// `[None, group]`: an optional group.
    Choice(Vec<Option<Item>>),
}

fn contains_group(item: &Item) -> bool {
    match item {
        Item::Group(..) => true,
        Item::NonCapture(v) => v.iter().any(contains_group),
        _ => false,
    }
}

fn escape_mapping(c: char) -> Option<Option<char>> {
    Some(match c {
        'A' | 'b' | 'B' | 'Z' | 'z' => None,
        'd' => Some('0'),
        'D' | 'S' | 'w' => Some('x'),
        's' => Some(' '),
        'W' => Some('!'),
        _ => return Some(Some(c)),
    })
}

/// Pre-tokenize like `next_char`: `(char, escaped)`, dropping zero-width escapes.
fn tokens(pattern: &str) -> Vec<(char, bool)> {
    let mut out = Vec::new();
    let mut it = pattern.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push((c, false));
            continue;
        }
        let Some(n) = it.next() else { break };
        if let Some(Some(rep)) = escape_mapping(n) {
            out.push((rep, true));
        }
    }
    out
}

struct Cursor {
    toks: Vec<(char, bool)>,
    pos: usize,
}

impl Cursor {
    fn next(&mut self) -> Option<(char, bool)> {
        let t = self.toks.get(self.pos).copied();
        self.pos += 1;
        t
    }

    /// `walk_to_end`: skip to the end of the group we are inside.
    fn walk_to_end(&mut self, ch: char) {
        let mut nesting = i32::from(ch == '(');
        while let Some((c, escaped)) = self.next() {
            if escaped {
                continue;
            }
            if c == '(' {
                nesting += 1;
            } else if c == ')' {
                if nesting == 0 {
                    return;
                }
                nesting -= 1;
            }
        }
    }

    /// `get_quantifier`: `(minimum count, next char if it is not part of the quantifier)`.
    fn quantifier(&mut self, ch: char) -> Result<(usize, Option<char>), String> {
        if "*?+".contains(ch) {
            let mut ch2 = self.next().map(|(c, _)| c);
            if ch2 == Some('?') {
                ch2 = None;
            }
            return Ok((usize::from(ch == '+'), ch2));
        }
        let mut quant = String::new();
        let mut c = ch;
        while c != '}' {
            c = self.next().ok_or("unterminated quantifier")?.0;
            quant.push(c);
        }
        quant.pop();
        let min = quant.split(',').next().unwrap_or("").parse::<usize>().map_err(|e| e.to_string())?;
        let mut next = self.next().map(|(c, _)| c);
        if next == Some('?') {
            next = None;
        }
        Ok((min, next))
    }
}

fn flatten_seq(source: &[Item]) -> (Vec<String>, Vec<Vec<String>>) {
    let mut result = vec![String::new()];
    let mut result_args: Vec<Vec<String>> = vec![vec![]];
    let mut last = 0;
    let mut pos = 0;
    let mut any_elt = false;
    for (p, elt) in source.iter().enumerate() {
        pos = p;
        if matches!(elt, Item::Ch(_)) {
            continue;
        }
        any_elt = true;
        let mut piece: String = source[last..p].iter().filter_map(|i| if let Item::Ch(c) = i { Some(*c) } else { None }).collect();
        let mut param = None;
        if let Item::Group(fmt, name) = elt {
            piece.push_str(fmt);
            param = name.clone();
        }
        last = p + 1;
        for (i, r) in result.iter_mut().enumerate() {
            r.push_str(&piece);
            if let Some(pn) = &param {
                result_args[i].push(pn.clone());
            }
        }
        let inner: Option<Vec<Option<Item>>> = match elt {
            Item::NonCapture(v) => Some(vec![Some(Item::NonCapture(v.clone()))]),
            Item::Choice(v) => Some(v.clone()),
            _ => None,
        };
        if let Some(items) = inner {
            let (mut inner_res, mut inner_args) = (Vec::new(), Vec::new());
            for item in &items {
                let (r, a) = flatten_item(item.as_ref());
                inner_res.extend(r);
                inner_args.extend(a);
            }
            let (mut new_res, mut new_args) = (Vec::new(), Vec::new());
            for (item, args) in result.iter().zip(&result_args) {
                for (ir, ia) in inner_res.iter().zip(&inner_args) {
                    new_res.push(format!("{item}{ir}"));
                    new_args.push(args.iter().chain(ia).cloned().collect());
                }
            }
            result = new_res;
            result_args = new_args;
        }
    }
    let _ = any_elt;
    if source.is_empty() || pos >= last {
        let piece: String = source[last..].iter().filter_map(|i| if let Item::Ch(c) = i { Some(*c) } else { None }).collect();
        for r in &mut result {
            r.push_str(&piece);
        }
    }
    (result, result_args)
}

fn flatten_item(item: Option<&Item>) -> (Vec<String>, Vec<Vec<String>>) {
    match item {
        None => (vec![String::new()], vec![vec![]]),
        Some(Item::Group(fmt, name)) => (vec![fmt.clone()], vec![name.iter().cloned().collect()]),
        Some(Item::NonCapture(v)) => flatten_seq(v),
        Some(Item::Ch(c)) => (vec![c.to_string()], vec![vec![]]),
        Some(Item::Choice(v)) => flatten_seq(&[Item::Choice(v.clone())]),
    }
}

/// `normalize(pattern)`: possible `(format string, parameter names)` pairs.
/// `Err` is the `ValueError` ("Non-reversible reg-exp portion").
pub fn normalize(pattern: &str) -> Result<Vec<(String, Vec<String>)>, String> {
    let mut cur = Cursor { toks: tokens(pattern), pos: 0 };
    let mut result: Vec<Item> = Vec::new();
    let mut non_capturing: Vec<usize> = Vec::new();
    let mut num_args = 0;
    let Some((mut ch, mut escaped)) = cur.next() else { return Ok(vec![(String::new(), vec![])]) };
    'outer: loop {
        let mut consume_next = true;
        macro_rules! next {
            () => {
                match cur.next() {
                    Some(t) => t,
                    None => break 'outer,
                }
            };
        }
        if escaped {
            result.push(Item::Ch(ch));
        } else {
            match ch {
                '.' => result.push(Item::Ch('.')),
                '|' => return Ok(vec![(String::new(), vec![])]),
                '^' => {}
                '$' => break,
                ')' => {
                    let Some(start) = non_capturing.pop() else { break };
                    let inner: Vec<Item> = result.split_off(start);
                    result.push(Item::NonCapture(inner));
                }
                '[' => {
                    let (c, _) = next!();
                    result.push(Item::Ch(c));
                    let (mut c, mut e) = next!();
                    while e || c != ']' {
                        (c, e) = next!();
                    }
                }
                '(' => {
                    let (c, e) = next!();
                    if c != '?' || e {
                        let name = format!("_{num_args}");
                        num_args += 1;
                        result.push(Item::Group(format!("%({name})s"), Some(name)));
                        cur.walk_to_end(c);
                    } else {
                        let (c, _) = next!();
                        if "!=<".contains(c) {
                            cur.walk_to_end(c);
                        } else if c == ':' {
                            non_capturing.push(result.len());
                        } else if c != 'P' {
                            return Err(format!("Non-reversible reg-exp portion: '(?{c}'"));
                        } else {
                            let (c, _) = next!();
                            if c != '<' && c != '=' {
                                return Err(format!("Non-reversible reg-exp portion: '(?P{c}'"));
                            }
                            let terminal = if c == '<' { '>' } else { ')' };
                            let mut name = String::new();
                            let (mut c, _) = next!();
                            while c != terminal {
                                name.push(c);
                                (c, _) = next!();
                            }
                            if terminal != ')' {
                                result.push(Item::Group(format!("%({name})s"), Some(name)));
                                cur.walk_to_end(c);
                            } else {
                                result.push(Item::Group(format!("%({name})s"), None));
                            }
                        }
                    }
                }
                '*' | '?' | '+' | '{' => {
                    let (count, next_ch) = cur.quantifier(ch)?;
                    if let Some(c) = next_ch {
                        ch = c; // note: `escaped` deliberately keeps its previous value
                        consume_next = false;
                    }
                    if count == 0 {
                        if let Some(last) = result.last().cloned() {
                            if contains_group(&last) {
                                *result.last_mut().expect("non-empty") = Item::Choice(vec![None, Some(last)]);
                            } else {
                                result.pop();
                            }
                        }
                    } else if count > 1 {
                        if let Some(last) = result.last().cloned() {
                            result.extend(std::iter::repeat_n(last, count - 1));
                        }
                    }
                }
                c => result.push(Item::Ch(c)),
            }
        }
        if consume_next {
            (ch, escaped) = next!();
        }
    }
    let (strings, args) = flatten_seq(&result);
    Ok(strings.into_iter().zip(args).collect())
}
