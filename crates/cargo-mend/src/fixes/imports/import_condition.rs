use std::collections::BTreeMap;
use std::collections::BTreeSet;

use quote::ToTokens;
use syn::Attribute;
use syn::Meta;
use syn::Token;
use syn::punctuated::Punctuated;

/// The configurations in which a binding exists. Conditions are compared by
/// meaning, so unrelated gates are preserved and overlapping gates can share
/// one import without losing any configuration.
#[derive(Clone, Eq, PartialEq)]
pub(super) enum ImportCondition {
    Always,
    Never,
    Atom(String),
    All(Vec<Self>),
    Any(Vec<Self>),
    Not(Box<Self>),
}

impl ImportCondition {
    pub(super) fn from_attributes(attributes: &[Attribute]) -> Self {
        Self::All(
            attributes
                .iter()
                .map(|attribute| Self::attribute(&attribute.meta))
                .collect(),
        )
    }

    fn attribute(meta: &Meta) -> Self {
        let Meta::List(list) = meta else {
            return Self::Always;
        };
        if list.path.is_ident("cfg") {
            return list
                .parse_args::<Meta>()
                .map_or(Self::Always, |meta| Self::predicate(&meta));
        }
        if list.path.is_ident("cfg_attr") {
            let Ok(args) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            else {
                return Self::Always;
            };
            let mut args = args.iter();
            let Some(predicate) = args.next() else {
                return Self::Always;
            };
            return Self::Any(vec![
                Self::Not(Box::new(Self::predicate(predicate))),
                Self::All(args.map(Self::attribute).collect()),
            ]);
        }
        Self::Always
    }

    fn predicate(meta: &Meta) -> Self {
        if let Meta::List(list) = meta
            && let Ok(args) = list.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
        {
            let args = args.iter().map(Self::predicate).collect::<Vec<_>>();
            if list.path.is_ident("all") {
                return Self::All(args);
            }
            if list.path.is_ident("any") {
                return Self::Any(args);
            }
            if list.path.is_ident("not") && args.len() == 1 {
                return Self::Not(Box::new(args.into_iter().next().unwrap_or(Self::Never)));
            }
        }
        Self::Atom(meta.to_token_stream().to_string())
    }

    pub(super) fn is_always(&self) -> bool { !Self::Not(Box::new(self.clone())).satisfiable() }

    pub(super) fn implies(&self, other: &Self) -> bool {
        !Self::All(vec![self.clone(), Self::Not(Box::new(other.clone()))]).satisfiable()
    }

    pub(super) fn equivalent(&self, other: &Self) -> bool {
        self.implies(other) && other.implies(self)
    }

    pub(super) fn overlaps(&self, other: &Self) -> bool {
        Self::All(vec![self.clone(), other.clone()]).satisfiable()
    }

    pub(super) fn union(&self, other: &Self) -> Self {
        if other.implies(self) {
            return self.clone();
        }
        if self.implies(other) {
            return other.clone();
        }
        let union = Self::Any(vec![self.clone(), other.clone()]);
        if union.is_always() {
            Self::Always
        } else {
            union
        }
    }

    pub(super) fn excluding(&self, other: &Self) -> Self {
        if !self.overlaps(other) {
            return self.clone();
        }
        Self::All(vec![self.clone(), Self::Not(Box::new(other.clone()))])
    }

    pub(super) fn render(&self) -> String {
        if self.is_always() {
            String::new()
        } else {
            format!("#[cfg({})]\n", self.predicate_text())
        }
    }

    fn predicate_text(&self) -> String {
        match self {
            Self::Always => "all()".to_string(),
            Self::Never => "any()".to_string(),
            Self::Atom(atom) => atom.clone(),
            Self::All(conditions) => format!(
                "all({})",
                conditions
                    .iter()
                    .map(Self::predicate_text)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Any(conditions) => format!(
                "any({})",
                conditions
                    .iter()
                    .map(Self::predicate_text)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::Not(condition) => format!("not({})", condition.predicate_text()),
        }
    }

    fn atoms(&self, atoms: &mut BTreeSet<String>) {
        match self {
            Self::Atom(atom) => {
                atoms.insert(atom.clone());
            },
            Self::All(conditions) | Self::Any(conditions) => {
                for condition in conditions {
                    condition.atoms(atoms);
                }
            },
            Self::Not(condition) => condition.atoms(atoms),
            Self::Always | Self::Never => {},
        }
    }

    fn evaluate(&self, values: &BTreeMap<String, bool>) -> Truth {
        match self {
            Self::Always => Truth::True,
            Self::Never => Truth::False,
            Self::Atom(atom) => values.get(atom).map_or(Truth::Unknown, |value| {
                if *value { Truth::True } else { Truth::False }
            }),
            Self::All(conditions) => {
                let mut result = Truth::True;
                for condition in conditions {
                    match condition.evaluate(values) {
                        Truth::False => return Truth::False,
                        Truth::Unknown => result = Truth::Unknown,
                        Truth::True => {},
                    }
                }
                result
            },
            Self::Any(conditions) => {
                let mut result = Truth::False;
                for condition in conditions {
                    match condition.evaluate(values) {
                        Truth::True => return Truth::True,
                        Truth::Unknown => result = Truth::Unknown,
                        Truth::False => {},
                    }
                }
                result
            },
            Self::Not(condition) => match condition.evaluate(values) {
                Truth::True => Truth::False,
                Truth::False => Truth::True,
                Truth::Unknown => Truth::Unknown,
            },
        }
    }

    fn satisfiable(&self) -> bool {
        let mut atoms = BTreeSet::new();
        self.atoms(&mut atoms);
        self.solve(&atoms.into_iter().collect::<Vec<_>>(), &mut BTreeMap::new())
    }

    fn solve(&self, atoms: &[String], values: &mut BTreeMap<String, bool>) -> bool {
        match self.evaluate(values) {
            Truth::True => true,
            Truth::False => false,
            Truth::Unknown => {
                let Some((atom, remaining)) = atoms.split_first() else {
                    return false;
                };
                for value in [false, true] {
                    values.insert(atom.clone(), value);
                    if self.solve(remaining, values) {
                        values.remove(atom);
                        return true;
                    }
                }
                values.remove(atom);
                false
            },
        }
    }
}

enum Truth {
    True,
    False,
    Unknown,
}
