use std::collections::HashMap;

use crate::types::Error;
use tree_sitter::{Node, Parser, Tree};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallPlan {
    pub actions: Vec<InstallAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallAction {
    Move {
        sources: Vec<String>,
        destination: InstallTarget,
    },
    Install {
        destination: InstallTarget,
        sources: Vec<InstallSource>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallSource {
    pub source: String,
    pub target_name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallTarget {
    Prefix,
    Bin,
    Sbin,
    Lib,
    Libexec,
    Include,
    Share,
    Man,
    Man1,
    Man2,
    Man3,
    Man4,
    Man5,
    Man6,
    Man7,
    Man8,
    Doc,
    Info,
    Pkgshare,
    BashCompletion,
    ZshCompletion,
    FishCompletion,
    Elisp,
    Frameworks,
    Kext,
}

#[derive(Debug)]
struct ParsedFormula<'a> {
    tree: Tree,
    source: &'a str,
}

impl<'a> ParsedFormula<'a> {
    fn install_method(&self) -> Option<Node<'_>> {
        find_install_method(self.tree.root_node(), self.source.as_bytes())
    }
}

pub fn parse_supported_install_plan(source: &str) -> Result<Option<InstallPlan>, Error> {
    let parsed = parse_formula(source)?;
    let Some(install) = parsed.install_method() else {
        return Ok(None);
    };

    let Some(body) = install.child_by_field_name("body") else {
        return Ok(Some(InstallPlan {
            actions: Vec::new(),
        }));
    };

    let mut locals = HashMap::new();
    let mut actions = Vec::new();
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        match child.kind() {
            "call" => {
                let Some(action) = parse_call(child, parsed.source.as_bytes(), &locals) else {
                    return Ok(None);
                };
                actions.push(action);
            }
            "assignment" => {
                let Some((name, value)) =
                    parse_assignment(child, parsed.source.as_bytes(), &locals)
                else {
                    return Ok(None);
                };
                locals.insert(name, value);
            }
            "unless_modifier" => {
                if !unless_modifier_is_skipped(child, parsed.source.as_bytes()) {
                    return Ok(None);
                }
            }
            "comment" => {}
            _ => return Ok(None),
        }
    }

    Ok(Some(InstallPlan { actions }))
}

fn parse_formula(source: &str) -> Result<ParsedFormula<'_>, Error> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_ruby::LANGUAGE.into())
        .map_err(|e| Error::ExecutionError {
            message: format!("failed to load Ruby grammar: {e}"),
        })?;

    let tree = parser
        .parse(source, None)
        .ok_or_else(|| Error::ExecutionError {
            message: "failed to parse Ruby formula".to_string(),
        })?;

    Ok(ParsedFormula { tree, source })
}

fn find_install_method<'a>(root: Node<'a>, source: &'a [u8]) -> Option<Node<'a>> {
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if (node.kind() == "method" || node.kind() == "method_definition")
            && let Some(name) = method_name(node, source)
            && name == "install"
        {
            return Some(node);
        }

        let mut cursor = node.walk();
        stack.extend(node.children(&mut cursor));
    }
    None
}

fn method_name<'a>(node: Node<'a>, source: &'a [u8]) -> Option<&'a str> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "constant" {
            return child.utf8_text(source).ok();
        }
    }
    None
}

fn parse_call(
    node: Node<'_>,
    source: &[u8],
    locals: &HashMap<String, String>,
) -> Option<InstallAction> {
    let method = node.child_by_field_name("method")?.utf8_text(source).ok()?;
    let receiver = node
        .child_by_field_name("receiver")
        .and_then(|value| value.utf8_text(source).ok());

    match (receiver, method) {
        (None, "mv") => parse_move_call(node, source),
        (Some(receiver), "install") => parse_install_call(receiver, node, source, locals),
        _ => None,
    }
}

fn parse_move_call(node: Node<'_>, source: &[u8]) -> Option<InstallAction> {
    let args = parse_arguments(node.child_by_field_name("arguments")?, source)?;
    let (destination, sources) = args.split_last()?;
    let destination = match destination {
        Argument::Target(target) => *target,
        Argument::String(_) | Argument::Renamed { .. } => return None,
    };

    let mut parsed_sources = Vec::with_capacity(sources.len());
    for source in sources {
        match source {
            Argument::String(value) => parsed_sources.push(value.clone()),
            Argument::Target(_) | Argument::Renamed { .. } => return None,
        }
    }

    Some(InstallAction::Move {
        sources: parsed_sources,
        destination,
    })
}

fn parse_install_call(
    receiver: &str,
    node: Node<'_>,
    source: &[u8],
    locals: &HashMap<String, String>,
) -> Option<InstallAction> {
    let destination = install_target(receiver)?;
    let args = parse_arguments_with_locals(node.child_by_field_name("arguments")?, source, locals)?;
    let mut sources = Vec::with_capacity(args.len());

    for arg in args {
        match arg {
            Argument::String(value) => sources.push(InstallSource {
                source: value,
                target_name: None,
            }),
            Argument::Renamed { source, target } => sources.push(InstallSource {
                source,
                target_name: Some(target),
            }),
            Argument::Target(_) => return None,
        }
    }

    Some(InstallAction::Install {
        destination,
        sources,
    })
}

fn parse_arguments(node: Node<'_>, source: &[u8]) -> Option<Vec<Argument>> {
    parse_arguments_with_locals(node, source, &HashMap::new())
}

fn parse_arguments_with_locals(
    node: Node<'_>,
    source: &[u8],
    locals: &HashMap<String, String>,
) -> Option<Vec<Argument>> {
    let mut values = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let parsed = match child.kind() {
            "string" => Argument::String(parse_string(child, source)?),
            "identifier" => {
                let identifier = child.utf8_text(source).ok()?;
                if let Some(value) = locals.get(identifier) {
                    Argument::String(value.clone())
                } else {
                    Argument::Target(install_target(identifier)?)
                }
            }
            "hash" => {
                let mut renamed = parse_hash_renames(child, source, locals)?;
                values.append(&mut renamed);
                continue;
            }
            "pair" => parse_rename_pair(child, source, locals)?,
            _ => return None,
        };
        values.push(parsed);
    }
    Some(values)
}

fn parse_assignment(
    node: Node<'_>,
    source: &[u8],
    locals: &HashMap<String, String>,
) -> Option<(String, String)> {
    let left = node.child_by_field_name("left")?;
    let name = left.utf8_text(source).ok()?.to_string();
    let value = eval_string_expr(node.child_by_field_name("right")?, source, locals)?;
    Some((name, value))
}

fn eval_string_expr(
    node: Node<'_>,
    source: &[u8],
    locals: &HashMap<String, String>,
) -> Option<String> {
    match node.kind() {
        "string" => parse_string(node, source),
        "identifier" => locals.get(node.utf8_text(source).ok()?).cloned(),
        "conditional" => {
            let condition = eval_bool_expr(node.child_by_field_name("condition")?, source)?;
            let branch = if condition {
                node.child_by_field_name("consequence")?
            } else {
                node.child_by_field_name("alternative")?
            };
            eval_string_expr(branch, source, locals)
        }
        _ => None,
    }
}

fn eval_bool_expr(node: Node<'_>, source: &[u8]) -> Option<bool> {
    match node.kind() {
        "true" => Some(true),
        "false" => Some(false),
        "call" => {
            let method = node.child_by_field_name("method")?.utf8_text(source).ok()?;
            let receiver = node
                .child_by_field_name("receiver")
                .and_then(|value| value.utf8_text(source).ok());
            match (receiver, method) {
                (Some("build"), "head?") => Some(false),
                (Some("build"), "stable?") => Some(true),
                (Some("OS"), "mac?") => Some(cfg!(target_os = "macos")),
                (Some("OS"), "linux?") => Some(cfg!(target_os = "linux")),
                _ => None,
            }
        }
        _ => None,
    }
}

fn unless_modifier_is_skipped(node: Node<'_>, source: &[u8]) -> bool {
    let Some(condition) = node.child_by_field_name("condition") else {
        return false;
    };
    if eval_bool_expr(condition, source) != Some(true) {
        return false;
    }

    let Some(body) = node.child_by_field_name("body") else {
        return false;
    };
    body.kind() == "call"
        && body
            .child_by_field_name("method")
            .and_then(|method| method.utf8_text(source).ok())
            == Some("odie")
}

fn parse_hash_renames(
    node: Node<'_>,
    source: &[u8],
    locals: &HashMap<String, String>,
) -> Option<Vec<Argument>> {
    let mut values = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() != "pair" {
            return None;
        }
        values.push(parse_rename_pair(child, source, locals)?);
    }
    Some(values)
}

fn parse_rename_pair(
    node: Node<'_>,
    source: &[u8],
    locals: &HashMap<String, String>,
) -> Option<Argument> {
    let from = eval_string_expr(node.child_by_field_name("key")?, source, locals)?;
    let to = eval_string_expr(node.child_by_field_name("value")?, source, locals)?;
    Some(Argument::Renamed {
        source: from,
        target: to,
    })
}

fn parse_string(node: Node<'_>, source: &[u8]) -> Option<String> {
    if let Some(content) = node.child_by_field_name("content")
        && let Ok(value) = content.utf8_text(source)
    {
        return Some(value.to_string());
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string_content" {
            return child.utf8_text(source).ok().map(ToString::to_string);
        }
    }

    Some(String::new())
}

fn install_target(value: &str) -> Option<InstallTarget> {
    match value {
        "prefix" => Some(InstallTarget::Prefix),
        "bin" => Some(InstallTarget::Bin),
        "sbin" => Some(InstallTarget::Sbin),
        "lib" => Some(InstallTarget::Lib),
        "libexec" => Some(InstallTarget::Libexec),
        "include" => Some(InstallTarget::Include),
        "share" => Some(InstallTarget::Share),
        "man" => Some(InstallTarget::Man),
        "man1" => Some(InstallTarget::Man1),
        "man2" => Some(InstallTarget::Man2),
        "man3" => Some(InstallTarget::Man3),
        "man4" => Some(InstallTarget::Man4),
        "man5" => Some(InstallTarget::Man5),
        "man6" => Some(InstallTarget::Man6),
        "man7" => Some(InstallTarget::Man7),
        "man8" => Some(InstallTarget::Man8),
        "doc" => Some(InstallTarget::Doc),
        "info" => Some(InstallTarget::Info),
        "pkgshare" => Some(InstallTarget::Pkgshare),
        "bash_completion" => Some(InstallTarget::BashCompletion),
        "zsh_completion" => Some(InstallTarget::ZshCompletion),
        "fish_completion" => Some(InstallTarget::FishCompletion),
        "elisp" => Some(InstallTarget::Elisp),
        "frameworks" => Some(InstallTarget::Frameworks),
        "kext" => Some(InstallTarget::Kext),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Argument {
    String(String),
    Target(InstallTarget),
    Renamed { source: String, target: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mv_into_prefix() {
        let plan = parse_supported_install_plan(
            r#"
class Foo < Formula
  def install
    mv "themes", prefix
  end
end
"#,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            plan,
            InstallPlan {
                actions: vec![InstallAction::Move {
                    sources: vec!["themes".to_string()],
                    destination: InstallTarget::Prefix,
                }],
            }
        );
    }

    #[test]
    fn parses_install_into_named_target() {
        let plan = parse_supported_install_plan(
            r#"
class Foo < Formula
  def install
    bin.install "foo"
    prefix.install "README.md", "LICENSE"
  end
end
"#,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            plan,
            InstallPlan {
                actions: vec![
                    InstallAction::Install {
                        destination: InstallTarget::Bin,
                        sources: vec![InstallSource {
                            source: "foo".to_string(),
                            target_name: None,
                        }],
                    },
                    InstallAction::Install {
                        destination: InstallTarget::Prefix,
                        sources: vec![
                            InstallSource {
                                source: "README.md".to_string(),
                                target_name: None,
                            },
                            InstallSource {
                                source: "LICENSE".to_string(),
                                target_name: None,
                            },
                        ],
                    },
                ],
            }
        );
    }

    #[test]
    fn parses_local_variable_ternary_and_renamed_install() {
        let plan = parse_supported_install_plan(
            r#"
class AgentSafehouse < Formula
  def install
    artifact_path = build.head? ? "dist/safehouse.sh" : "safehouse.sh"
    bin.install artifact_path => "safehouse"
  end
end
"#,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            plan,
            InstallPlan {
                actions: vec![InstallAction::Install {
                    destination: InstallTarget::Bin,
                    sources: vec![InstallSource {
                        source: "safehouse.sh".to_string(),
                        target_name: Some("safehouse".to_string()),
                    }],
                }],
            }
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn skips_macos_odie_guard() {
        let plan = parse_supported_install_plan(
            r#"
class AgentSafehouse < Formula
  def install
    odie "Agent Safehouse requires macOS" unless OS.mac?
    bin.install "safehouse.sh" => "safehouse"
  end
end
"#,
        )
        .unwrap()
        .unwrap();

        assert_eq!(
            plan,
            InstallPlan {
                actions: vec![InstallAction::Install {
                    destination: InstallTarget::Bin,
                    sources: vec![InstallSource {
                        source: "safehouse.sh".to_string(),
                        target_name: Some("safehouse".to_string()),
                    }],
                }],
            }
        );
    }

    #[test]
    fn returns_none_for_unsupported_system_calls() {
        let plan = parse_supported_install_plan(
            r#"
class Foo < Formula
  def install
    system "sh", "-c", "echo hi"
  end
end
"#,
        )
        .unwrap();

        assert!(plan.is_none());
    }
}
