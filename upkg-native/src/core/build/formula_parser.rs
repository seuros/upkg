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
        sources: Vec<String>,
    },
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

    let mut actions = Vec::new();
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if child.kind() == "call" {
            let Some(action) = parse_call(child, parsed.source.as_bytes()) else {
                return Ok(None);
            };
            actions.push(action);
            continue;
        }

        if child.kind() == "comment" {
            continue;
        }

        return Ok(None);
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

fn parse_call(node: Node<'_>, source: &[u8]) -> Option<InstallAction> {
    let method = node.child_by_field_name("method")?.utf8_text(source).ok()?;
    let receiver = node
        .child_by_field_name("receiver")
        .and_then(|value| value.utf8_text(source).ok());

    match (receiver, method) {
        (None, "mv") => parse_move_call(node, source),
        (Some(receiver), "install") => parse_install_call(receiver, node, source),
        _ => None,
    }
}

fn parse_move_call(node: Node<'_>, source: &[u8]) -> Option<InstallAction> {
    let args = parse_arguments(node.child_by_field_name("arguments")?, source)?;
    let (destination, sources) = args.split_last()?;
    let destination = match destination {
        Argument::Target(target) => *target,
        Argument::String(_) => return None,
    };

    let mut parsed_sources = Vec::with_capacity(sources.len());
    for source in sources {
        match source {
            Argument::String(value) => parsed_sources.push(value.clone()),
            Argument::Target(_) => return None,
        }
    }

    Some(InstallAction::Move {
        sources: parsed_sources,
        destination,
    })
}

fn parse_install_call(receiver: &str, node: Node<'_>, source: &[u8]) -> Option<InstallAction> {
    let destination = install_target(receiver)?;
    let args = parse_arguments(node.child_by_field_name("arguments")?, source)?;
    let mut sources = Vec::with_capacity(args.len());

    for arg in args {
        match arg {
            Argument::String(value) => sources.push(value),
            Argument::Target(_) => return None,
        }
    }

    Some(InstallAction::Install {
        destination,
        sources,
    })
}

fn parse_arguments(node: Node<'_>, source: &[u8]) -> Option<Vec<Argument>> {
    let mut values = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        let parsed = match child.kind() {
            "string" => Argument::String(parse_string(child, source)?),
            "identifier" => Argument::Target(install_target(child.utf8_text(source).ok()?)?),
            _ => return None,
        };
        values.push(parsed);
    }
    Some(values)
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
                        sources: vec!["foo".to_string()],
                    },
                    InstallAction::Install {
                        destination: InstallTarget::Prefix,
                        sources: vec!["README.md".to_string(), "LICENSE".to_string()],
                    },
                ],
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
