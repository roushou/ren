//! The API's wire types as the compatibility contract reads them: each
//! type ts-rs writes to the UI (`ui/apps/web/src/lib/wire`), parsed into a
//! [`Shape`], and compared with the base by structure, not by text. Whether
//! a difference breaks depends on which way the type travels: an install's
//! older clients still read its responses and still send it their
//! requests. The gateway pins which types are requests
//! (`crates/ren-gateway/tests/fixtures/requests.txt`); a type reached from
//! one is a request, a type reached from any other is a response, and a
//! type reached from both is both.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

/// Where ts-rs writes the wire types, from the repository's root.
pub const WIRE: &str = "ui/apps/web/src/lib/wire";
/// The gateway's request types, one name a line.
pub const REQUESTS: &str = "crates/ren-gateway/tests/fixtures/requests.txt";

/// A type as the wire carries it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shape {
    /// A named type, with its type arguments: a declared wire type, a
    /// primitive (`string`, `number`, `boolean`, `null`, `unknown`), or a
    /// built-in such as `Array<T>`.
    Named(String, Vec<Shape>),
    /// A string or number literal, as written (`"image"`, `3`).
    Literal(String),
    Object(Object),
    Union(Vec<Shape>),
    Intersection(Vec<Shape>),
    Tuple(Vec<Shape>),
}

/// An object type: its fields, and the values of its index signature, if
/// it has one (a map).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Object {
    fields: BTreeMap<String, Field>,
    index: Option<Box<Shape>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    presence: Presence,
    shape: Shape,
}

/// Whether a field must be there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presence {
    Required,
    /// Written `name?:`; it may be left out.
    Optional,
}

/// Which way a type travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Sent by clients, read by the daemon.
    Request,
    /// Sent by the daemon, read by clients.
    Response,
    Both,
}

/// A difference between the base's wire types and the change's, at one
/// place (`CreateRequest.dir`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiChange {
    pub at: String,
    pub direction: Direction,
    pub difference: Difference,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Difference {
    TypeAdded,
    TypeRemoved,
    FieldAdded(Presence),
    FieldRemoved(Presence),
    NowOptional,
    NowRequired,
    NullAdded,
    NullRemoved,
    LiteralAdded(String),
    LiteralRemoved(String),
    VariantAdded(String),
    VariantRemoved(String),
    MemberAdded(String),
    MemberRemoved(String),
    Retyped {
        from: String,
        to: String,
    },
    /// A wire file the contract could not read: never taken as unchanged.
    Unreadable(String),
}

impl ApiChange {
    /// Whether an install's older clients still work: they read what a
    /// response now holds, and the daemon reads what they still send.
    pub fn breaks(&self) -> bool {
        let response = || self.difference.breaks_readers();
        let request = || self.difference.breaks_senders();
        match self.direction {
            Direction::Response => response(),
            Direction::Request => request(),
            Direction::Both => response() || request(),
        }
    }
}

impl Difference {
    /// Whether an older client reading this response fails. A new label,
    /// variant, or field is kept or passed over by readers, as labels are.
    fn breaks_readers(&self) -> bool {
        match self {
            Self::TypeRemoved
            | Self::FieldRemoved(Presence::Required)
            | Self::NowOptional
            | Self::NullAdded
            | Self::MemberAdded(_)
            | Self::Retyped { .. }
            | Self::Unreadable(_) => true,
            Self::TypeAdded
            | Self::FieldAdded(_)
            | Self::FieldRemoved(Presence::Optional)
            | Self::NowRequired
            | Self::NullRemoved
            | Self::LiteralAdded(_)
            | Self::LiteralRemoved(_)
            | Self::VariantAdded(_)
            | Self::VariantRemoved(_)
            | Self::MemberRemoved(_) => false,
        }
    }

    /// Whether the daemon refuses what an older client sends. Requests
    /// deny unknown fields, so a field taken away breaks too.
    fn breaks_senders(&self) -> bool {
        match self {
            Self::TypeRemoved
            | Self::FieldAdded(Presence::Required)
            | Self::FieldRemoved(_)
            | Self::NowRequired
            | Self::NullRemoved
            | Self::LiteralRemoved(_)
            | Self::VariantRemoved(_)
            | Self::MemberRemoved(_)
            | Self::Retyped { .. }
            | Self::Unreadable(_) => true,
            Self::TypeAdded
            | Self::FieldAdded(Presence::Optional)
            | Self::NowOptional
            | Self::NullAdded
            | Self::LiteralAdded(_)
            | Self::VariantAdded(_)
            | Self::MemberAdded(_) => false,
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Request => "request",
            Self::Response => "response",
            Self::Both => "request and response",
        })
    }
}

impl fmt::Display for ApiChange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let at = &self.at;
        let what = match &self.difference {
            Difference::TypeAdded => "type added".to_owned(),
            Difference::TypeRemoved => "type removed".to_owned(),
            Difference::FieldAdded(Presence::Required) => "required field added".to_owned(),
            Difference::FieldAdded(Presence::Optional) => "optional field added".to_owned(),
            Difference::FieldRemoved(_) => "field removed".to_owned(),
            Difference::NowOptional => "now optional".to_owned(),
            Difference::NowRequired => "now required".to_owned(),
            Difference::NullAdded => "may now be null".to_owned(),
            Difference::NullRemoved => "no longer null".to_owned(),
            Difference::LiteralAdded(literal) => format!("gained {literal}"),
            Difference::LiteralRemoved(literal) => format!("lost {literal}"),
            Difference::VariantAdded(tag) => format!("variant {tag} added"),
            Difference::VariantRemoved(tag) => format!("variant {tag} removed"),
            Difference::MemberAdded(member) => format!("may now be {member}"),
            Difference::MemberRemoved(member) => format!("is no longer {member}"),
            Difference::Retyped { from, to } => format!("was {from}, now {to}"),
            Difference::Unreadable(reason) => format!("could not be read: {reason}"),
        };
        write!(f, "{at}: {what} ({})", self.direction)
    }
}

impl fmt::Display for Shape {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |f: &mut fmt::Formatter<'_>, shapes: &[Shape], separator: &str| {
            for (position, shape) in shapes.iter().enumerate() {
                if position > 0 {
                    f.write_str(separator)?;
                }
                write!(f, "{shape}")?;
            }
            Ok(())
        };
        match self {
            Self::Named(name, arguments) => {
                f.write_str(name)?;
                if !arguments.is_empty() {
                    f.write_str("<")?;
                    list(f, arguments, ", ")?;
                    f.write_str(">")?;
                }
                Ok(())
            }
            Self::Literal(literal) => f.write_str(literal),
            Self::Object(object) => {
                f.write_str("{ ")?;
                for (name, field) in &object.fields {
                    let mark = match field.presence {
                        Presence::Required => "",
                        Presence::Optional => "?",
                    };
                    write!(f, "{name}{mark}: {}, ", field.shape)?;
                }
                if let Some(index) = &object.index {
                    write!(f, "[key]: {index}, ")?;
                }
                f.write_str("}")
            }
            Self::Union(members) => list(f, members, " | "),
            Self::Intersection(parts) => list(f, parts, " & "),
            Self::Tuple(items) => {
                f.write_str("[")?;
                list(f, items, ", ")?;
                f.write_str("]")
            }
        }
    }
}

/// One token of a ts-rs declaration.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Word(String),
    Text(String),
    Number(String),
    Mark(char),
}

/// Why a wire file could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct Unreadable(String);

/// A parser over one declaration's tokens.
struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Shape {
    /// The type a ts-rs file declares: its name and shape. Imports and
    /// comments are skipped.
    pub fn declared(source: &str) -> Result<(String, Self), Unreadable> {
        let code = Self::without_comments(source);
        let start = code
            .find("export type ")
            .ok_or_else(|| Unreadable("no `export type`".into()))?;
        let tokens = Self::tokens(code.get(start..).unwrap_or_default())?;
        let mut parser = Parser { tokens, at: 0 };
        parser.expect_word("export")?;
        parser.expect_word("type")?;
        let name = parser.word()?;
        parser.expect(&Token::Mark('='))?;
        let shape = parser.shape()?;
        parser.expect(&Token::Mark(';'))?;
        Ok((name, shape))
    }

    fn without_comments(source: &str) -> String {
        let mut out = String::new();
        let mut rest = source;
        loop {
            let line = rest.find("//");
            let block = rest.find("/*");
            let (cut, end) = match (line, block) {
                (Some(l), Some(b)) if l < b => (l, "\n"),
                (Some(l), None) => (l, "\n"),
                (_, Some(b)) => (b, "*/"),
                (None, None) => {
                    out.push_str(rest);
                    return out;
                }
            };
            out.push_str(rest.get(..cut).unwrap_or_default());
            let after = rest.get(cut..).unwrap_or_default();
            rest = after
                .find(end)
                .and_then(|position| after.get(position + end.len()..))
                .unwrap_or_default();
            out.push(' ');
        }
    }

    fn tokens(code: &str) -> Result<Vec<Token>, Unreadable> {
        let mut tokens = Vec::new();
        let mut chars = code.chars().peekable();
        while let Some(&c) = chars.peek() {
            if c.is_whitespace() {
                chars.next();
            } else if c == '"' {
                chars.next();
                let mut text = String::new();
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => {
                            text.push('\\');
                            text.extend(chars.next());
                        }
                        Some(other) => text.push(other),
                        None => return Err(Unreadable("an unclosed string".into())),
                    }
                }
                tokens.push(Token::Text(text));
            } else if c.is_ascii_digit() || c == '-' {
                let mut number = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() || d == '.' || d == '-' {
                        number.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Number(number));
            } else if c.is_alphanumeric() || c == '_' || c == '$' {
                let mut word = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_alphanumeric() || d == '_' || d == '$' {
                        word.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                tokens.push(Token::Word(word));
            } else if "{}[]()<>,:;|&?=".contains(c) {
                tokens.push(Token::Mark(c));
                chars.next();
            } else {
                return Err(Unreadable(format!("an unexpected `{c}`")));
            }
        }
        Ok(tokens)
    }

    /// Every declared type this one names.
    fn names(&self, into: &mut BTreeSet<String>) {
        match self {
            Self::Named(name, arguments) => {
                into.insert(name.clone());
                for argument in arguments {
                    argument.names(into);
                }
            }
            Self::Literal(_) => {}
            Self::Object(object) => {
                for field in object.fields.values() {
                    field.shape.names(into);
                }
                if let Some(index) = &object.index {
                    index.names(into);
                }
            }
            Self::Union(shapes) | Self::Intersection(shapes) | Self::Tuple(shapes) => {
                for shape in shapes {
                    shape.names(into);
                }
            }
        }
    }
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn next(&mut self) -> Result<Token, Unreadable> {
        let token = self
            .tokens
            .get(self.at)
            .cloned()
            .ok_or_else(|| Unreadable("the declaration ends early".into()))?;
        self.at += 1;
        Ok(token)
    }

    fn eat(&mut self, token: &Token) -> bool {
        let found = self.peek() == Some(token);
        if found {
            self.at += 1;
        }
        found
    }

    fn expect(&mut self, token: &Token) -> Result<(), Unreadable> {
        let found = self.next()?;
        if &found == token {
            Ok(())
        } else {
            Err(Unreadable(format!("expected {token:?}, found {found:?}")))
        }
    }

    fn word(&mut self) -> Result<String, Unreadable> {
        match self.next()? {
            Token::Word(word) => Ok(word),
            other @ (Token::Text(_) | Token::Number(_) | Token::Mark(_)) => {
                Err(Unreadable(format!("expected a name, found {other:?}")))
            }
        }
    }

    fn expect_word(&mut self, word: &str) -> Result<(), Unreadable> {
        let found = self.word()?;
        if found == word {
            Ok(())
        } else {
            Err(Unreadable(format!("expected `{word}`, found `{found}`")))
        }
    }

    fn shape(&mut self) -> Result<Shape, Unreadable> {
        self.eat(&Token::Mark('|'));
        let mut members = vec![self.intersection()?];
        while self.eat(&Token::Mark('|')) {
            members.push(self.intersection()?);
        }
        Ok(Self::single(members, Shape::Union))
    }

    fn intersection(&mut self) -> Result<Shape, Unreadable> {
        let mut parts = vec![self.primary()?];
        while self.eat(&Token::Mark('&')) {
            parts.push(self.primary()?);
        }
        Ok(Self::single(parts, Shape::Intersection))
    }

    fn single(mut shapes: Vec<Shape>, many: fn(Vec<Shape>) -> Shape) -> Shape {
        match shapes.pop() {
            Some(only) if shapes.is_empty() => only,
            Some(last) => {
                shapes.push(last);
                many(shapes)
            }
            None => many(shapes),
        }
    }

    fn primary(&mut self) -> Result<Shape, Unreadable> {
        match self.next()? {
            Token::Mark('{') => self.object(),
            Token::Mark('[') => {
                let items = self.list(']')?;
                Ok(Shape::Tuple(items))
            }
            Token::Mark('(') => {
                let shape = self.shape()?;
                self.expect(&Token::Mark(')'))?;
                Ok(shape)
            }
            Token::Text(text) => Ok(Shape::Literal(format!("\"{text}\""))),
            Token::Number(number) => Ok(Shape::Literal(number)),
            Token::Word(name) => {
                let arguments = if self.eat(&Token::Mark('<')) {
                    self.list('>')?
                } else {
                    Vec::new()
                };
                Ok(Shape::Named(name, arguments))
            }
            Token::Mark(other) => Err(Unreadable(format!("an unexpected `{other}`"))),
        }
    }

    /// Shapes separated by commas, up to `close`.
    fn list(&mut self, close: char) -> Result<Vec<Shape>, Unreadable> {
        let mut shapes = Vec::new();
        while !self.eat(&Token::Mark(close)) {
            shapes.push(self.shape()?);
            if !self.eat(&Token::Mark(',')) {
                self.expect(&Token::Mark(close))?;
                break;
            }
        }
        Ok(shapes)
    }

    fn object(&mut self) -> Result<Shape, Unreadable> {
        let mut object = Object::default();
        while !self.eat(&Token::Mark('}')) {
            if self.eat(&Token::Mark('[')) {
                // `[key in string]?: T`
                self.word()?;
                self.expect_word("in")?;
                self.shape()?;
                self.expect(&Token::Mark(']'))?;
                self.eat(&Token::Mark('?'));
                self.expect(&Token::Mark(':'))?;
                object.index = Some(Box::new(self.shape()?));
            } else {
                let name = match self.next()? {
                    Token::Word(name) | Token::Text(name) => name,
                    other @ (Token::Number(_) | Token::Mark(_)) => {
                        return Err(Unreadable(format!("expected a field, found {other:?}")));
                    }
                };
                let presence = if self.eat(&Token::Mark('?')) {
                    Presence::Optional
                } else {
                    Presence::Required
                };
                self.expect(&Token::Mark(':'))?;
                let shape = self.shape()?;
                object.fields.insert(name, Field { presence, shape });
            }
            if !self.eat(&Token::Mark(',')) {
                self.expect(&Token::Mark('}'))?;
                break;
            }
        }
        Ok(Shape::Object(object))
    }
}

/// A union, sorted out: whether it may be null, its literals, its tagged
/// variants (by their tag, `kind="image"`), and its other members.
#[derive(Default)]
struct Members<'a> {
    null: bool,
    literals: BTreeSet<String>,
    variants: BTreeMap<String, &'a Shape>,
    others: Vec<&'a Shape>,
}

impl<'a> Members<'a> {
    fn of(shape: &'a Shape) -> Self {
        let mut members = Self::default();
        let all = match shape {
            Shape::Union(all) => all.iter().collect(),
            Shape::Named(..)
            | Shape::Literal(_)
            | Shape::Object(_)
            | Shape::Intersection(_)
            | Shape::Tuple(_) => vec![shape],
        };
        for member in all {
            match member {
                Shape::Named(name, arguments) if name == "null" && arguments.is_empty() => {
                    members.null = true;
                }
                Shape::Literal(literal) => {
                    members.literals.insert(literal.clone());
                }
                Shape::Named(..)
                | Shape::Object(_)
                | Shape::Intersection(_)
                | Shape::Tuple(_)
                | Shape::Union(_) => match Self::tag(member) {
                    Some(tag) => {
                        members.variants.insert(tag, member);
                    }
                    None => members.others.push(member),
                },
            }
        }
        members
    }

    /// A variant's tag: the first field of its object (or of its
    /// intersection's first object) whose type is a literal.
    fn tag(shape: &Shape) -> Option<String> {
        let object = match shape {
            Shape::Object(object) => object,
            Shape::Intersection(parts) => match parts.first() {
                Some(Shape::Object(object)) => object,
                Some(_) | None => return None,
            },
            Shape::Named(..) | Shape::Literal(_) | Shape::Union(_) | Shape::Tuple(_) => {
                return None;
            }
        };
        object
            .fields
            .iter()
            .find_map(|(name, field)| match &field.shape {
                Shape::Literal(literal) => Some(format!("{name}={literal}")),
                Shape::Named(..)
                | Shape::Object(_)
                | Shape::Union(_)
                | Shape::Intersection(_)
                | Shape::Tuple(_) => None,
            })
    }
}

/// One revision's wire types, by name, and the gateway's request types.
#[derive(Debug, Default)]
pub struct WireTypes {
    types: BTreeMap<String, Shape>,
    requests: BTreeSet<String>,
    unreadable: Vec<(String, Unreadable)>,
}

impl WireTypes {
    /// Read a wire file at `path`.
    pub fn file(&mut self, path: &str, source: &str) {
        match Shape::declared(source) {
            Ok((name, shape)) => {
                self.types.insert(name, shape);
            }
            Err(reason) => self.unreadable.push((path.to_owned(), reason)),
        }
    }

    /// Read the gateway's request types.
    pub fn requests(&mut self, list: &str) {
        self.requests = list
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect();
    }

    /// Each type's direction: reached from a request type, from any other
    /// type nothing reaches through a request, or both.
    fn directions(&self) -> BTreeMap<String, Direction> {
        let reach = |roots: Vec<String>| {
            let mut seen = BTreeSet::new();
            let mut stack = roots;
            while let Some(name) = stack.pop() {
                if let Some(shape) = self.types.get(&name)
                    && seen.insert(name)
                {
                    let mut named = BTreeSet::new();
                    shape.names(&mut named);
                    stack.extend(named);
                }
            }
            seen
        };
        let requests = reach(self.requests.iter().cloned().collect());
        let responses = reach(
            self.types
                .keys()
                .filter(|name| !requests.contains(*name))
                .cloned()
                .collect(),
        );
        self.types
            .keys()
            .map(|name| {
                let direction = match (requests.contains(name), responses.contains(name)) {
                    (true, true) => Direction::Both,
                    (true, false) => Direction::Request,
                    (false, true | false) => Direction::Response,
                };
                (name.clone(), direction)
            })
            .collect()
    }

    /// The types nothing else names: what a route sends or reads whole.
    fn roots(&self) -> BTreeSet<&String> {
        let mut named = BTreeSet::new();
        for shape in self.types.values() {
            shape.names(&mut named);
        }
        self.types
            .keys()
            .filter(|name| self.requests.contains(*name) || !named.contains(*name))
            .collect()
    }

    /// What changed from `base` to `head`.
    pub fn compare(base: &Self, head: &Self) -> Vec<ApiChange> {
        let mut changes = Vec::new();
        let (before, after) = (base.directions(), head.directions());
        for (path, reason) in &head.unreadable {
            changes.push(ApiChange {
                at: path.clone(),
                direction: Direction::Both,
                difference: Difference::Unreadable(reason.to_string()),
            });
        }
        let base_roots = base.roots();
        for name in base.types.keys() {
            if !head.types.contains_key(name) && base_roots.contains(name) {
                changes.push(ApiChange {
                    at: name.clone(),
                    direction: before.get(name).copied().unwrap_or(Direction::Both),
                    difference: Difference::TypeRemoved,
                });
            }
        }
        for name in head.roots() {
            if !base.types.contains_key(name) {
                changes.push(ApiChange {
                    at: name.clone(),
                    direction: after.get(name).copied().unwrap_or(Direction::Both),
                    difference: Difference::TypeAdded,
                });
            }
        }
        for (name, old) in &base.types {
            let Some(new) = head.types.get(name) else {
                continue;
            };
            let direction = match (before.get(name), after.get(name)) {
                (Some(a), Some(b)) if a == b => *a,
                (Some(_) | None, Some(_) | None) => Direction::Both,
            };
            let mut comparing = Comparing {
                base,
                head,
                direction,
                changes: &mut changes,
                resolving: BTreeSet::new(),
            };
            comparing.shapes(name, old, new);
        }
        changes
    }
}

/// One type's comparison under way.
struct Comparing<'a> {
    base: &'a WireTypes,
    head: &'a WireTypes,
    direction: Direction,
    changes: &'a mut Vec<ApiChange>,
    /// Renamed types being compared by what they hold, so a recursive type
    /// is compared once.
    resolving: BTreeSet<(String, String)>,
}

impl Comparing<'_> {
    fn found(&mut self, at: &str, difference: Difference) {
        self.changes.push(ApiChange {
            at: at.to_owned(),
            direction: self.direction,
            difference,
        });
    }

    fn shapes(&mut self, at: &str, old: &Shape, new: &Shape) {
        if old == new {
            return;
        }
        let (before, after) = (Members::of(old), Members::of(new));
        match (before.null, after.null) {
            (false, true) => self.found(at, Difference::NullAdded),
            (true, false) => self.found(at, Difference::NullRemoved),
            (true, true) | (false, false) => {}
        }
        for literal in before.literals.difference(&after.literals) {
            self.found(at, Difference::LiteralRemoved(literal.clone()));
        }
        for literal in after.literals.difference(&before.literals) {
            self.found(at, Difference::LiteralAdded(literal.clone()));
        }
        for (tag, old_variant) in &before.variants {
            match after.variants.get(tag) {
                Some(new_variant) => self.single(&format!("{at}[{tag}]"), old_variant, new_variant),
                None => self.found(at, Difference::VariantRemoved(tag.clone())),
            }
        }
        for tag in after.variants.keys() {
            if !before.variants.contains_key(tag) {
                self.found(at, Difference::VariantAdded(tag.clone()));
            }
        }
        match (before.others.as_slice(), after.others.as_slice()) {
            ([old], [new]) => self.single(at, old, new),
            (old, new) => {
                let text = |shapes: &[&Shape]| {
                    shapes
                        .iter()
                        .map(ToString::to_string)
                        .collect::<BTreeSet<_>>()
                };
                let (old, new) = (text(old), text(new));
                for member in old.difference(&new) {
                    self.found(at, Difference::MemberRemoved(member.clone()));
                }
                for member in new.difference(&old) {
                    self.found(at, Difference::MemberAdded(member.clone()));
                }
            }
        }
    }

    /// Two shapes in the same place that are neither null nor literals.
    fn single(&mut self, at: &str, old: &Shape, new: &Shape) {
        if old == new {
            return;
        }
        match (old, new) {
            (Shape::Object(old), Shape::Object(new)) => self.objects(at, old, new),
            (Shape::Intersection(old), Shape::Intersection(new)) if old.len() == new.len() => {
                for (old, new) in old.iter().zip(new) {
                    self.single(at, old, new);
                }
            }
            (Shape::Named(old_name, old_args), Shape::Named(new_name, new_args))
                if old_name == new_name && old_args.len() == new_args.len() =>
            {
                for (old, new) in old_args.iter().zip(new_args) {
                    self.shapes(&format!("{at}<>"), old, new);
                }
            }
            (Shape::Named(old_name, old_args), Shape::Named(new_name, new_args))
                if old_args.is_empty() && new_args.is_empty() =>
            {
                // A renamed type carries the same JSON when what it holds
                // is the same.
                match (self.base.types.get(old_name), self.head.types.get(new_name)) {
                    (Some(old), Some(new)) => {
                        if self.resolving.insert((old_name.clone(), new_name.clone())) {
                            self.shapes(at, old, new);
                        }
                    }
                    (Some(_) | None, Some(_) | None) => self.retyped(at, old, new),
                }
            }
            (Shape::Named(..), _)
            | (Shape::Literal(_), _)
            | (Shape::Object(_), _)
            | (Shape::Union(_), _)
            | (Shape::Intersection(_), _)
            | (Shape::Tuple(_), _) => self.retyped(at, old, new),
        }
    }

    fn retyped(&mut self, at: &str, old: &Shape, new: &Shape) {
        self.found(
            at,
            Difference::Retyped {
                from: old.to_string(),
                to: new.to_string(),
            },
        );
    }

    fn objects(&mut self, at: &str, old: &Object, new: &Object) {
        for (name, field) in &old.fields {
            let place = format!("{at}.{name}");
            match new.fields.get(name) {
                None => self.found(&place, Difference::FieldRemoved(field.presence)),
                Some(now) => {
                    match (field.presence, now.presence) {
                        (Presence::Required, Presence::Optional) => {
                            self.found(&place, Difference::NowOptional);
                        }
                        (Presence::Optional, Presence::Required) => {
                            self.found(&place, Difference::NowRequired);
                        }
                        (Presence::Required, Presence::Required)
                        | (Presence::Optional, Presence::Optional) => {}
                    }
                    self.shapes(&place, &field.shape, &now.shape);
                }
            }
        }
        for (name, field) in &new.fields {
            if !old.fields.contains_key(name) {
                self.found(
                    &format!("{at}.{name}"),
                    Difference::FieldAdded(field.presence),
                );
            }
        }
        match (&old.index, &new.index) {
            (Some(old), Some(new)) => self.shapes(&format!("{at}[key]"), old, new),
            (None, None) => {}
            (Some(_), None) | (None, Some(_)) => {
                self.retyped(at, &Shape::Object(old.clone()), &Shape::Object(new.clone()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiChange, Difference, Direction, Presence, Shape, WireTypes};

    fn types(files: &[&str], requests: &str) -> WireTypes {
        let mut types = WireTypes::default();
        for (position, source) in files.iter().enumerate() {
            types.file(&format!("wire/{position}.ts"), source);
        }
        types.requests(requests);
        types
    }

    fn compare(base: &[&str], head: &[&str], requests: &str) -> Vec<(String, Difference, bool)> {
        WireTypes::compare(&types(base, requests), &types(head, requests))
            .into_iter()
            .map(|change| {
                let breaks = change.breaks();
                (change.at, change.difference, breaks)
            })
            .collect()
    }

    #[test]
    fn a_ts_rs_file_reads_as_its_shape() {
        let source = r#"// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { DeviceCode } from "./DeviceCode.ts";

/**
 * A login, as it goes.
 */
export type Login = { "kind": "starting", said: string | null, } | { "kind": "awaiting_approval", link: string, code: DeviceCode | null, said?: string | null, } | { "kind": "image" } & Image;
"#;
        let (name, shape) = Shape::declared(source).unwrap();
        assert_eq!(name, "Login");
        assert_eq!(
            shape.to_string(),
            r#"{ kind: "starting", said: string | null, } | { code: DeviceCode | null, kind: "awaiting_approval", link: string, said?: string | null, } | { kind: "image", } & Image"#
        );
        for source in [
            "export type Accepted = Record<symbol, never>;",
            "export type Map = { blocked: { [key in string]?: BlockedUntil }, };",
            "export type JsonValue = number | string | boolean | Array<JsonValue> | { [key in string]: JsonValue } | null;",
            "export type Pair = [string, number];",
        ] {
            assert!(Shape::declared(source).is_ok(), "{source}");
        }
        for broken in [
            "",
            "export type X = { a: string, ",
            "export type X = string",
            "export type X = @;",
        ] {
            assert!(Shape::declared(broken).is_err(), "{broken:?}");
        }
    }

    const BODY: &str = "export type CreateRequest = { harness: string, dir?: string | null, settings: RunSettings, };";
    const SETTINGS: &str = "export type RunSettings = { model: string | null, };";
    const VIEW: &str = "export type SessionView = { id: string, state: \"draft\" | \"active\", settings: RunSettings, };";

    #[test]
    fn directions_follow_the_request_types() {
        let types = types(&[BODY, SETTINGS, VIEW], "CreateRequest\n");
        let directions = types.directions();
        assert_eq!(directions["CreateRequest"], Direction::Request);
        assert_eq!(directions["SessionView"], Direction::Response);
        assert_eq!(directions["RunSettings"], Direction::Both);
    }

    #[test]
    fn a_response_may_grow_but_not_lose_what_readers_expect() {
        let base = [VIEW, SETTINGS];
        let with = |view: &str| compare(&base, &[view, SETTINGS], "");
        assert_eq!(
            with(
                "export type SessionView = { id: string, state: \"draft\" | \"active\" | \"archived\", settings: RunSettings, title: string, };"
            ),
            [
                (
                    "SessionView.state".into(),
                    Difference::LiteralAdded("\"archived\"".into()),
                    false
                ),
                (
                    "SessionView.title".into(),
                    Difference::FieldAdded(Presence::Required),
                    false
                ),
            ]
        );
        assert_eq!(
            with(
                "export type SessionView = { state: \"draft\" | \"active\", settings: RunSettings, };"
            ),
            [(
                "SessionView.id".into(),
                Difference::FieldRemoved(Presence::Required),
                true
            )]
        );
        assert_eq!(
            with(
                "export type SessionView = { id: string | null, state: \"draft\" | \"active\", settings: RunSettings, };"
            ),
            [("SessionView.id".into(), Difference::NullAdded, true)]
        );
        assert_eq!(
            with(
                "export type SessionView = { id: number, state: \"draft\" | \"active\", settings: RunSettings, };"
            ),
            [(
                "SessionView.id".into(),
                Difference::Retyped {
                    from: "string".into(),
                    to: "number".into()
                },
                true
            )]
        );
    }

    #[test]
    fn a_request_may_take_more_but_never_need_more() {
        let base = [BODY, SETTINGS];
        let with = |body: &str| compare(&base, &[body, SETTINGS], "CreateRequest");
        assert_eq!(
            with(
                "export type CreateRequest = { harness: string, dir?: string | null, settings: RunSettings, resume?: string | null, };"
            ),
            [(
                "CreateRequest.resume".into(),
                Difference::FieldAdded(Presence::Optional),
                false
            )]
        );
        assert_eq!(
            with(
                "export type CreateRequest = { harness: string, dir?: string | null, settings: RunSettings, place: string, };"
            ),
            [(
                "CreateRequest.place".into(),
                Difference::FieldAdded(Presence::Required),
                true
            )]
        );
        assert_eq!(
            with(
                "export type CreateRequest = { harness: string, dir: string | null, settings: RunSettings, };"
            ),
            [("CreateRequest.dir".into(), Difference::NowRequired, true)]
        );
        // Requests deny unknown fields: an older client still sends it.
        assert_eq!(
            with("export type CreateRequest = { harness: string, settings: RunSettings, };"),
            [(
                "CreateRequest.dir".into(),
                Difference::FieldRemoved(Presence::Optional),
                true
            )]
        );
    }

    #[test]
    fn a_type_both_ways_keeps_both_rules() {
        let changes = WireTypes::compare(
            &types(&[BODY, SETTINGS, VIEW], "CreateRequest"),
            &types(
                &[
                    BODY,
                    "export type RunSettings = { model: string | null, effort: string, };",
                    VIEW,
                ],
                "CreateRequest",
            ),
        );
        assert_eq!(
            changes,
            [ApiChange {
                at: "RunSettings.effort".into(),
                direction: Direction::Both,
                difference: Difference::FieldAdded(Presence::Required),
            }]
        );
        assert!(changes[0].breaks(), "an older client sends no effort");
    }

    #[test]
    fn tagged_variants_are_compared_by_their_tag() {
        let base = [
            "export type RunState = { \"phase\": \"idle\", thread: string, } | { \"phase\": \"working\", turn: string | null, };",
        ];
        assert_eq!(
            compare(
                &base,
                &[
                    "export type RunState = { \"phase\": \"idle\", thread: string, } | { \"phase\": \"working\", turn: string | null, approvals: number, } | { \"phase\": \"stopping\" };"
                ],
                ""
            ),
            [
                (
                    "RunState[phase=\"working\"].approvals".into(),
                    Difference::FieldAdded(Presence::Required),
                    false
                ),
                (
                    "RunState".into(),
                    Difference::VariantAdded("phase=\"stopping\"".into()),
                    false
                ),
            ]
        );
        assert_eq!(
            compare(
                &base,
                &["export type RunState = { \"phase\": \"idle\", thread: string, };"],
                "RunState"
            ),
            [(
                "RunState".into(),
                Difference::VariantRemoved("phase=\"working\"".into()),
                true
            )]
        );
    }

    #[test]
    fn renaming_a_type_is_no_change_when_it_holds_the_same() {
        let base = [VIEW, SETTINGS];
        let head = [
            "export type SessionView = { id: string, state: \"draft\" | \"active\", settings: Settings, };",
            "export type Settings = { model: string | null, };",
        ];
        assert!(compare(&base, &head, "").is_empty());
        let removed = compare(&[VIEW, SETTINGS, BODY], &[VIEW, SETTINGS], "CreateRequest");
        assert_eq!(
            removed,
            [("CreateRequest".into(), Difference::TypeRemoved, true)]
        );
    }

    #[test]
    fn an_unreadable_wire_file_is_never_taken_as_unchanged() {
        let changes = compare(&[VIEW], &[VIEW, "export type Broken = {"], "");
        assert_eq!(changes.len(), 1);
        assert!(matches!(changes[0].1, Difference::Unreadable(_)));
        assert!(changes[0].2);
    }
}
