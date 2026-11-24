//! Resolve built renames in the finding's crate.

use std::collections::HashMap;

use super::approvals::ApprovalSnapshot;
use super::approvals::ReadAnswers;
use super::approvals::Unobserved;
use super::built;
use super::built::BuiltRename;
use super::built::BuiltScope;
use super::built::ItemRenameReading;
use crate::constants::OWNER_KEYWORDS;
use crate::declaration_source::DeclarationSource;
use crate::declaration_source::SourceLocation;
use crate::declarations::DeclaredItem;
use crate::declarations::ImplMembership;
use crate::finding_source::FindingSource;
use crate::finding_source::FindingSourceFailure;
use crate::finding_source::NamingSearch;
use crate::ledger_read::FindingEdit;
use crate::same_declaration::Declaration;
use crate::same_declaration::Owner;
use crate::tip_view::RepositoryTip;
use crate::trait_identity;
use crate::trait_identity::RequiredTrait;
use crate::trait_identity::TraitHome;
use crate::trait_identity::TraitIdentity;
use crate::trait_identity::TraitMembership;
use crate::trait_identity::TraitPath;
use crate::trait_identity::TraitReader;
use crate::trait_identity::TraitRequirement;
use crate::trait_identity::TraitSite;

/// Resolve the built renames that apply to one declaration.
pub(super) struct BuiltRenames<'a, 's> {
    pub(super) source:       &'a mut FindingSource<'s>,
    pub(super) declarations: &'a mut DeclarationSource,
    pub(super) traits:       &'a mut TraitReader,
    pub(super) approvals:    &'a ApprovalSnapshot,
    pub(super) answers:      &'a mut ReadAnswers,
    pub(super) renames:      &'a [BuiltRename],
    pub(super) edit:         &'a FindingEdit,
    pub(super) site:         SourceLocation<'a>,
    pub(super) tip:          &'a RepositoryTip,
}

impl BuiltRenames<'_, '_> {
    /// The new names that the built renames of approved decisions in the finding's crate gave its
    /// item and the impl or trait that holds it.
    pub(super) fn resolve(
        &mut self,
        old: &DeclaredItem,
        declaration: &Declaration,
    ) -> Result<BuiltRenameResolution, BuiltRenameError> {
        let mut requirement = TraitRequirement::of(old, self.site);
        let crate_path = self.tip.crate_of(self.site.path);
        // A built finding can sit at a call site in any file of the crate; the gaining-file search
        // still guards it.
        let mut chain = Renaming {
            in_crate:   self
                .renames
                .iter()
                .filter(|rename| self.tip.crate_of(&rename.path) == crate_path)
                .collect(),
            own:        built::item_rename(self.edit),
            module:     built::declaring_module(self.site.path),
            questions:  RenameQuestions::NoneAsked,
            uncertain:  Vec::new(),
            references: HashMap::new(),
            owners:     Vec::new(),
            traits:     Vec::new(),
        };
        let Owner::ImplOrTraitItem { name: owner } = declaration.owner() else {
            let names = self.renamed(&mut chain, declaration.name(), Holder::Module)?;
            let [_, .., last] = names.as_slice() else {
                return Ok(chain.questions.unrenamed().into());
            };
            return Ok(AfterBuilds::Renamed {
                declaration: declaration.renamed(last, Owner::ModuleItem),
                requirement,
            }
            .into());
        };
        // The impl's type and its implemented trait have separate name histories.
        chain.owners = self.renamed(&mut chain, owner, Holder::Owner(owner))?;
        if let ImplMembership::Path(implemented) = old.implemented_trait() {
            let trait_name = implemented.path.last().cloned().unwrap_or_default();
            chain.references.insert(
                trait_name.clone(),
                RequiredTrait {
                    membership: old.implemented_trait().into(),
                    commit:     self.site.commit.to_owned(),
                    path:       self.site.path.to_owned(),
                },
            );
            chain.traits = self.renamed(&mut chain, &trait_name, Holder::Trait(&trait_name))?;
            if let Some(reference) = chain
                .traits
                .last()
                .and_then(|name| chain.references.get(name))
            {
                requirement = TraitRequirement::Required(reference.clone());
            }
        }
        let names = self.renamed(&mut chain, declaration.name(), Holder::ImplOrTrait)?;
        let last_name = names
            .last()
            .map_or_else(|| declaration.name(), String::as_str);
        let last_owner = chain.owners.last().map_or(owner.as_str(), String::as_str);
        let renamed_owner = Owner::ImplOrTraitItem {
            name: last_owner.to_owned(),
        };
        if !chain.uncertain.is_empty() {
            let candidates = chain.uncertain.iter().map(|rename| {
                let name = match rename.scope {
                    BuiltScope::TypeQualified(_) => rename.new.as_str(),
                    BuiltScope::Unqualified | BuiltScope::ModuleQualified(_) => last_name,
                };
                declaration.renamed(name, renamed_owner.clone())
            });
            return Ok(BuiltRenameResolution::NeedsDestinationProof(
                candidates.collect(),
            ));
        }
        if names.len() == 1 && chain.owners.len() == 1 && chain.traits.len() <= 1 {
            return Ok(chain.questions.unrenamed().into());
        }
        Ok(AfterBuilds::Renamed {
            declaration: declaration.renamed(last_name, renamed_owner),
            requirement,
        }
        .into())
    }

    /// The name, then each new name the renames gave it in turn, up to a name with two new names or
    /// a name the chain already holds; the name alone when the finding's own rename names it.
    fn renamed(
        &mut self,
        chain: &mut Renaming<'_>,
        name: &str,
        holder: Holder<'_>,
    ) -> Result<Vec<String>, BuiltRenameError> {
        let mut names = vec![name.to_owned()];
        let mut questions = self.questions(chain, name, holder)?;
        if matches!(&chain.own, ItemRenameReading::Renamed(own) if own.old == name) {
            return Ok(names);
        }
        while !questions.is_empty() {
            let mut new = Vec::new();
            for question in &questions {
                let approved = self
                    .approvals
                    .approved(&question.proposal, self.answers)
                    .map_err(BuiltRenameError::Approval)?;
                if approved && !new.contains(&question.new) {
                    new.push(question.new.clone());
                }
            }
            let [next] = new.as_slice() else {
                return Ok(names);
            };
            if names.contains(next) {
                return Ok(names);
            }
            names.push(next.clone());
            questions = self.questions(chain, next, holder)?;
        }
        Ok(names)
    }

    fn questions<'r>(
        &mut self,
        chain: &mut Renaming<'r>,
        name: &str,
        holder: Holder<'_>,
    ) -> Result<Vec<&'r BuiltRename>, BuiltRenameError> {
        let named: Vec<_> = chain
            .in_crate
            .iter()
            .copied()
            .filter(|rename| rename.old == name)
            .collect();
        let mut questions = Vec::new();
        for rename in named {
            if self.holds(chain, rename, holder)? {
                questions.push(rename);
            }
        }
        if !questions.is_empty() {
            chain.questions = RenameQuestions::Asked;
        }
        Ok(questions)
    }

    fn holds<'r>(
        &mut self,
        chain: &mut Renaming<'r>,
        rename: &'r BuiltRename,
        holder: Holder<'_>,
    ) -> Result<bool, BuiltRenameError> {
        let module_item = match &rename.scope {
            BuiltScope::Unqualified => true,
            BuiltScope::ModuleQualified(scope) => *scope == chain.module,
            BuiltScope::TypeQualified(_) => false,
        };
        match holder {
            Holder::Module => Ok(module_item),
            Holder::Owner(owner) => {
                if module_item {
                    return Ok(true);
                }
                let BuiltScope::ModuleQualified(scope) = &rename.scope else {
                    return Ok(false);
                };
                self.declares_owner(owner, scope)
            },
            Holder::Trait(trait_name) => {
                let path = match &rename.scope {
                    BuiltScope::Unqualified => vec![rename.old.clone()],
                    BuiltScope::ModuleQualified(scope) => vec![scope.clone(), rename.old.clone()],
                    BuiltScope::TypeQualified(_) => return Ok(false),
                };
                let matches = self.trait_matches(chain, rename, &path, trait_name)?;
                if matches {
                    grant_new_trait_name(chain, rename);
                }
                Ok(matches)
            },
            Holder::ImplOrTrait => match &rename.scope {
                BuiltScope::Unqualified => Ok(true),
                BuiltScope::ModuleQualified(_) => Ok(false),
                BuiltScope::TypeQualified(type_name) => match chain.traits.first().cloned() {
                    // The chain's first trait name is the implemented trait's own name.
                    Some(trait_name) if chain.traits.contains(type_name) => self.trait_matches(
                        chain,
                        rename,
                        std::slice::from_ref(type_name),
                        &trait_name,
                    ),
                    _ => Ok(chain.owners.contains(type_name)),
                },
            },
        }
    }

    /// Whether a file of the module in the finding's crate declares the owner as a module type.
    fn declares_owner(&mut self, owner: &str, module: &str) -> Result<bool, BuiltRenameError> {
        let crate_path = self.tip.crate_of(self.site.path);
        let named =
            self.source
                .paths_naming(self.site.commit, owner, NamingSearch::DeclarationProof)?;
        for path in named.iter() {
            if self.tip.crate_of(path) != crate_path || built::declaring_module(path) != module {
                continue;
            }
            let location = SourceLocation {
                commit: self.site.commit,
                path,
            };
            let file = self.declarations.file(self.source, location)?;
            if file.items().iter().any(|item| {
                item.name == owner
                    && OWNER_KEYWORDS.contains(&item.keyword())
                    && item.is_module_item()
            }) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether the trait that a built rename's qualifier names is the item's implemented trait.
    fn trait_matches<'r>(
        &mut self,
        chain: &mut Renaming<'r>,
        rename: &'r BuiltRename,
        path: &[String],
        trait_name: &str,
    ) -> Result<bool, BuiltRenameError> {
        let last = path.last().map_or("", String::as_str);
        let Some(reference) = chain
            .references
            .get(last)
            .or_else(|| chain.references.get(trait_name))
            .cloned()
        else {
            return Ok(false);
        };
        let TraitMembership::Path {
            path: trait_path,
            arguments,
            ..
        } = &reference.membership
        else {
            return Ok(false);
        };
        let mut identity = self.trait_rename_identity(&reference, arguments, rename, path)?;
        if matches!(identity, TraitIdentity::Uncertain)
            && path.len() == 1
            && trait_path.last().map(String::as_str) == Some(last)
            && last != trait_name
        {
            // A proved rename grants its new name before a later build declares it. Imports still
            // decide that name first, and another declared trait prevents this chain from proving
            // identity.
            let before = SourceLocation {
                commit: &rename.before_commit,
                path:   &rename.before_path,
            };
            let resolved = self.traits.site_path(
                self.source,
                self.declarations,
                before,
                path,
                (&rename.before_position).into(),
            )?;
            let declared = self.declares_trait(&rename.before_commit, last)?;
            if matches!(&resolved, TraitPath::Resolved(resolved) if resolved.as_slice() == path)
                && !declared
            {
                identity = TraitIdentity::Same;
            }
        }
        if matches!(identity, TraitIdentity::Uncertain) {
            chain.uncertain.push(rename);
        }
        Ok(matches!(identity, TraitIdentity::Same))
    }

    /// Whether a Rust file of the finding's crate at the tip declares the trait as a module item at
    /// `commit`.
    fn declares_trait(&mut self, commit: &str, name: &str) -> Result<bool, BuiltRenameError> {
        let crate_path = self.tip.crate_of(self.site.path);
        let named = self
            .source
            .paths_naming(commit, name, NamingSearch::DeclarationProof)?;
        for path in named.iter() {
            if !self.tip.has_rust_path(path) || self.tip.crate_of(path) != crate_path {
                continue;
            }
            let file = self
                .declarations
                .file(self.source, SourceLocation { commit, path })?;
            if file
                .items()
                .iter()
                .any(|item| item.keyword() == "trait" && item.name == name && item.is_module_item())
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Prove what the qualifier names at the built finding's own site and commit.
    fn trait_rename_identity(
        &mut self,
        reference: &RequiredTrait,
        arguments: &[String],
        rename: &BuiltRename,
        qualifier: &[String],
    ) -> Result<TraitIdentity, BuiltRenameError> {
        let before = SourceLocation {
            commit: &rename.before_commit,
            path:   &rename.before_path,
        };
        let TraitPath::Resolved(mut candidate) = self.traits.site_path(
            self.source,
            self.declarations,
            before,
            qualifier,
            (&rename.before_position).into(),
        )?
        else {
            return Ok(TraitIdentity::Uncertain);
        };
        let same_site =
            reference.commit == rename.before_commit && reference.path == rename.before_path;
        if !same_site && let [name] = candidate.as_slice() {
            let TraitHome::Resolved { module, .. } =
                trait_identity::trait_home(self.source, self.declarations, before, &candidate)?
            else {
                return Ok(TraitIdentity::Uncertain);
            };
            let name = name.clone();
            candidate = if module.is_empty() {
                vec![String::new()]
            } else {
                module
            };
            candidate.push(name);
        }
        let membership = TraitMembership::Path {
            path:      candidate,
            arguments: arguments.to_vec(),
            position:  rename.before_position.clone(),
        };
        let right = TraitSite {
            location:   before,
            membership: (&membership).into(),
        };
        Ok(self
            .traits
            .compare(self.source, self.declarations, reference.site(), right)?)
    }
}

/// What the built renames of approved decisions did to the finding's item.
pub(super) enum AfterBuilds {
    Renamed {
        declaration: Declaration,
        requirement: TraitRequirement,
    },
    NoRename,
    Refused,
}

/// The built rename result or the possible declarations that need a destination search.
pub(super) enum BuiltRenameResolution {
    Settled(AfterBuilds),
    NeedsDestinationProof(Vec<Declaration>),
}

impl From<AfterBuilds> for BuiltRenameResolution {
    fn from(after: AfterBuilds) -> Self { Self::Settled(after) }
}

pub(super) enum BuiltRenameError {
    Approval(Unobserved),
    Source(FindingSourceFailure),
}

impl From<FindingSourceFailure> for BuiltRenameError {
    fn from(failure: FindingSourceFailure) -> Self { Self::Source(failure) }
}

/// Whether any built rename asked about a name in the chain.
#[derive(Clone, Copy)]
enum RenameQuestions {
    Asked,
    NoneAsked,
}

/// The built renames that can name a part of the item: its name, its owner, or its trait.
#[derive(Clone, Copy)]
enum Holder<'n> {
    Module,
    Owner(&'n str),
    Trait(&'n str),
    ImplOrTrait,
}

/// The chain state of one built-renaming read.
struct Renaming<'r> {
    in_crate:   Vec<&'r BuiltRename>,
    own:        ItemRenameReading,
    module:     String,
    questions:  RenameQuestions,
    uncertain:  Vec<&'r BuiltRename>,
    references: HashMap<String, RequiredTrait>,
    owners:     Vec<String>,
    traits:     Vec<String>,
}

/// The trait's new name belongs to the build commit; its before name belongs to the read commit.
fn grant_new_trait_name(chain: &mut Renaming<'_>, rename: &BuiltRename) {
    let Some(RequiredTrait {
        membership: TraitMembership::Path {
            path, arguments, ..
        },
        ..
    }) = chain.references.get(&rename.old)
    else {
        return;
    };
    let mut path = path.clone();
    path.pop();
    path.push(rename.new.clone());
    let membership = TraitMembership::Path {
        path,
        arguments: arguments.clone(),
        position: rename.position.clone(),
    };
    chain.references.insert(
        rename.new.clone(),
        RequiredTrait {
            membership,
            commit: rename.commit.clone(),
            path: rename.path.clone(),
        },
    );
}

impl RenameQuestions {
    /// A chain with no new name refuses the rename when a built rename asked about it.
    const fn unrenamed(self) -> AfterBuilds {
        match self {
            Self::Asked => AfterBuilds::Refused,
            Self::NoneAsked => AfterBuilds::NoRename,
        }
    }
}
