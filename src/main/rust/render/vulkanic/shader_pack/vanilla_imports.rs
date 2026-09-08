//! Bounded Mojang imports over immutable copied source data, without file or
//! renderer access. Import-once and quoted-prefix rules follow Frozen's GLSL
//! preprocessor; namespaces not represented by the snapshot fail closed.

use super::source::ShaderPackSource;
use crate::render::vulkanic::error::{GalError, GalResult};
use std::collections::BTreeSet;

const MAX_DEPTH: usize = 32;
const MAX_EXPANDED_BYTES: usize = 4 * 1024 * 1024;

pub(super) fn expand(
    source: &ShaderPackSource,
    namespace: &str,
    path: &str,
    contents: &str,
) -> GalResult<String> {
    let directory = path.rsplit_once('/').map_or("", |(directory, _)| directory);
    let mut context = Imports {
        source,
        namespace,
        directory,
        seen: BTreeSet::new(),
        version: 0,
        emitted: 0,
        qualified: super::vanilla_sources::qualified(source)?,
    };
    let body = context.process(contents, "", 0, false)?;
    if context.version == 0 {
        return Ok(body);
    }
    // Only replace the root directive; imported directives became comments.
    let clean = without_comments(&body)?;
    let mut output = String::new();
    for (line, visible) in body.split_inclusive('\n').zip(clean.split_inclusive('\n')) {
        if directive(visible, "version").is_some() {
            output.push_str(&format!("#version {}\n", context.version));
        } else {
            output.push_str(line);
        }
    }
    if output.len() > MAX_EXPANDED_BYTES {
        return Err(GalError::unsupported_feature(
            "expanded Mojang shader exceeds byte bound",
        ));
    }
    Ok(output)
}

struct Imports<'a> {
    source: &'a ShaderPackSource,
    namespace: &'a str,
    directory: &'a str,
    seen: BTreeSet<String>,
    version: u32,
    emitted: usize,
    qualified: bool,
}

impl Imports<'_> {
    fn process(
        &mut self,
        contents: &str,
        prefix: &str,
        depth: usize,
        imported: bool,
    ) -> GalResult<String> {
        if depth > MAX_DEPTH {
            return Err(GalError::unsupported_feature(
                "Mojang import depth exceeds bound",
            ));
        }
        let clean = without_comments(contents)?;
        let mut output = String::new();
        for visible in clean.split_inclusive('\n') {
            if let Some(argument) = directive(visible, "moj_import") {
                let argument = argument.trim();
                let (quoted, name) = if argument.len() >= 2
                    && argument.starts_with('"')
                    && argument.ends_with('"')
                {
                    (true, &argument[1..argument.len() - 1])
                } else if argument.starts_with('<') && argument.ends_with('>') {
                    (false, &argument[1..argument.len() - 1])
                } else {
                    return Err(GalError::unsupported_feature(
                        "unsupported or malformed Mojang import directive",
                    ));
                };
                if name.is_empty() || name.starts_with('/') {
                    return Err(GalError::invalid_argument(
                        "empty or absolute Mojang import path",
                    ));
                }
                let name = format!("{prefix}{name}");
                let (namespace, path) = if quoted {
                    (self.namespace, normalize_path(&format!("{}/{name}", self.directory))?)
                } else {
                    let (namespace, path) = name.split_once(':').unwrap_or(("minecraft", &name));
                    if !self.qualified && namespace != self.namespace {
                        return Err(GalError::unsupported_feature("cross-namespace Mojang import requires a namespace-qualified source snapshot"));
                    }
                    (namespace, normalize_path(&format!("include/{path}"))?)
                };
                let path = if self.qualified { super::vanilla_sources::key(namespace, &path)? } else { path };
                if self.seen.insert(path.clone()) {
                    let included = self.source.get(&path).ok_or_else(|| {
                        GalError::unsupported_feature(format!(
                            "Mojang import '{path}' is missing from the copied source snapshot"
                        ))
                    })?;
                    let next_prefix = if quoted {
                        name.rsplit_once('/')
                            .map_or(String::new(), |(parent, _)| format!("{parent}/"))
                    } else {
                        String::new()
                    };
                    output.push_str(&self.process(included, &next_prefix, depth + 1, true)?);
                }
                self.push(&mut output, "\n")?;
            } else if let Some(argument) = directive(visible, "version") {
                let words = argument.split_whitespace().collect::<Vec<_>>();
                if words.len() != 1 {
                    return Err(GalError::unsupported_feature(
                        "Mojang GLSL version profiles require an explicit source contract",
                    ));
                }
                let version = words[0]
                    .parse::<u32>()
                    .map_err(|_| GalError::invalid_argument("invalid Mojang GLSL version"))?;
                self.version = self.version.max(version);
                if imported {
                    self.push(&mut output, "// imported GLSL version\n")?;
                } else {
                    self.push(&mut output, visible)?;
                }
            } else {
                // Emit the comment-masked line, not a partial original block
                // comment whose closing delimiter may be on an import line.
                self.push(&mut output, visible)?;
            }
        }
        Ok(output)
    }

    fn push(&mut self, output: &mut String, text: &str) -> GalResult<()> {
        self.emitted = self
            .emitted
            .checked_add(text.len())
            .ok_or_else(|| GalError::unsupported_feature("Mojang import size overflow"))?;
        if self.emitted > MAX_EXPANDED_BYTES {
            return Err(GalError::unsupported_feature(
                "expanded Mojang shader exceeds byte bound",
            ));
        }
        output.push_str(text);
        Ok(())
    }
}

fn directive<'a>(line: &'a str, keyword: &str) -> Option<&'a str> {
    let tail = line
        .trim_start()
        .strip_prefix('#')?
        .trim_start()
        .strip_prefix(keyword)?;
    if tail.is_empty() || tail.starts_with(char::is_whitespace) {
        Some(tail)
    } else {
        None
    }
}

fn normalize_path(path: &str) -> GalResult<String> {
    let mut parts = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(GalError::invalid_argument(
                        "Mojang import escapes shader root",
                    ));
                }
            }
            _ if part
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"_.-".contains(&c)) =>
            {
                parts.push(part)
            }
            _ => return Err(GalError::invalid_argument("invalid Mojang import path")),
        }
    }
    if parts.is_empty() {
        return Err(GalError::invalid_argument("empty Mojang import path"));
    }
    Ok(parts.join("/"))
}

fn directive_after_comments(mut tail: &str, keyword: &str) -> bool {
    loop {
        tail = tail.trim_start_matches([' ', '\t']);
        if let Some(comment) = tail.strip_prefix("/*") {
            let Some((_, rest)) = comment.split_once("*/") else { return false; };
            tail = rest;
        } else {
            return tail.strip_prefix(keyword).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with("/*")
            });
        }
    }
}

fn without_comments(source: &str) -> GalResult<String> {
    let bytes = source.as_bytes();
    let mut output = bytes.to_vec();
    let mut i = 0;
    let mut block = false;
    let mut line = false;
    let mut line_start = 0;
    let mut join_block_lines = false;
    let mut version_block_lines = false;
    while i < bytes.len() {
        if bytes[i] == b'\n' {
            if block && version_block_lines {
                return Err(GalError::unsupported_feature("multiline GLSL version directives are not admitted by the vanilla source contract"));
            }
            if block && join_block_lines {
                output[i] = b' ';
            } else {
                line_start = i + 1;
            }
            line = false;
            i += 1;
            continue;
        }
        if !block && !line && bytes.get(i..i + 2) == Some(b"//") {
            line = true;
        }
        if !line && !block && bytes.get(i..i + 2) == Some(b"/*") {
            // Frozen's Mojang directive grammar permits a complete C comment
            // (including newlines) in either token gap. Join only those gaps;
            // never join physical newlines or comments in ordinary GLSL code.
            let prefix = std::str::from_utf8(&output[line_start..i])
                .expect("comment masking preserves UTF-8");
            join_block_lines = (prefix.trim() == "#" && directive_after_comments(&source[i..], "moj_import"))
                || directive(prefix, "moj_import").is_some_and(|tail| tail.trim().is_empty());
            // Frozen leaves version-token comments for the OpenGL compiler,
            // unlike imports which it expands itself. The real paired version
            // fixture was rejected there; do not normalize it into valid GLSL.
            version_block_lines = (prefix.trim() == "#" && directive_after_comments(&source[i..], "version"))
                || directive(prefix, "version").is_some_and(|tail| tail.trim().is_empty());
            block = true;
            output[i] = b' ';
            output[i + 1] = b' ';
            i += 2;
            continue;
        }
        if block && bytes.get(i..i + 2) == Some(b"*/") {
            block = false;
            output[i] = b' ';
            output[i + 1] = b' ';
            i += 2;
            continue;
        }
        if block || line {
            output[i] = b' ';
        }
        i += 1;
    }
    if block {
        return Err(GalError::invalid_argument(
            "unterminated Mojang shader block comment",
        ));
    }
    Ok(String::from_utf8(output).expect("comment masking preserves UTF-8 outside comments"))
}

#[cfg(test)]
mod tests {
    use super::super::source::ShaderSourceFile;
    use super::*;

    fn snapshot(files: &[(&str, &str)]) -> ShaderPackSource {
        ShaderPackSource::new(
            "copied-import-fixture",
            1,
            files
                .iter()
                .map(|(path, text)| ShaderSourceFile::new(*path, *text))
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn multiline_comments_join_only_mojang_directive_token_gaps() {
        let source = snapshot(&[("include/a.glsl", "float imported;\n")]);
        let result = expand(&source, "minecraft", "post/test.fsh",
            "#version 330\n#/* five\n six */moj_import/* seven\n eight */<a.glsl>\nfloat ordinary; /* keep\n line */\n").unwrap();
        assert!(result.starts_with("#version 330\n"));
        assert_eq!(1, result.matches("float imported;").count());
        assert!(!result.contains("moj_import"));
        assert_eq!("float a;     \n       float b;\n", without_comments("float a; /* x\n y */  float b;\n").unwrap());
        assert_eq!("#\nmoj_import <a.glsl>\n", without_comments("#\nmoj_import <a.glsl>\n").unwrap());
        assert_eq!("#       \n      define VALUE 1\n", without_comments("#/* keep\nme */ define VALUE 1\n").unwrap());
        assert!(!directive_after_comments("/* joined\n*/\nmoj_import <a.glsl>", "moj_import"));
        assert!(!directive_after_comments("/* joined\n*/moj_import_invalid <a.glsl>", "moj_import"));
        for version in ["#/* gap\n*/version 330\n", "#version/* gap\n*/330\n"] {
            assert!(expand(&source, "minecraft", "post/test.fsh", version).unwrap_err().message.contains("multiline GLSL version"));
        }
        assert!(expand(&source, "minecraft", "post/test.fsh", "#moj_import\n<a.glsl>\n").is_err());
        assert!(expand(&source, "minecraft", "post/test.fsh", "#moj_import /* unterminated\n").is_err());
    }

    #[test]
    fn nested_quoted_imports_use_root_directory_and_accumulated_prefix() {
        let source = snapshot(&[
            (
                "post/lib/a.glsl",
                "#moj_import \"nested/b.glsl\"\nfloat a;\n",
            ),
            ("post/lib/nested/b.glsl", "float b;\n"),
        ]);
        let result = expand(
            &source,
            "minecraft",
            "post/test.fsh",
            "#version 330\n#moj_import \"lib/a.glsl\"\nvoid main(){}\n",
        )
        .unwrap();
        assert!(result.find("float b;").unwrap() < result.find("float a;").unwrap());
        assert!(!result.contains("#moj_import"));
    }

    #[test]
    fn angle_import_resets_quoted_prefix_to_original_shader_directory() {
        let source = snapshot(&[
            ("include/a.glsl", "#moj_import \"local.glsl\"\n"),
            ("post/local.glsl", "float originalDirectory;\n"),
            ("include/local.glsl", "float wrongDirectory;\n"),
        ]);
        let result = expand(
            &source,
            "minecraft",
            "post/test.fsh",
            "#version 330\n#moj_import <minecraft:a.glsl>\n",
        )
        .unwrap();
        assert!(result.contains("float originalDirectory;"));
        assert!(!result.contains("wrongDirectory"));
    }

    #[test]
    fn versions_comments_and_import_once_match_stage_local_contract() {
        let source = snapshot(&[
            (
                "include/a.glsl",
                "#version 450\n#moj_import <b.glsl>\nfloat a;\n",
            ),
            (
                "include/b.glsl",
                "#version 400\n#moj_import <a.glsl>\nfloat b;\n",
            ),
        ]);
        let root = "#version 330\n/* #moj_import <missing.glsl> */\n// #moj_import <missing.glsl>\n# /* note */ moj_import <a.glsl> // trailing\n#moj_import <minecraft:a.glsl>\n";
        let result = expand(&source, "minecraft", "post/test.fsh", root).unwrap();
        assert!(result.starts_with("#version 450\n"));
        assert_eq!(1, result.matches("float a;").count());
        assert_eq!(1, result.matches("float b;").count());
        assert_eq!(1, result.matches("#version").count());
        assert_eq!(
            result,
            expand(&source, "minecraft", "post/test.fsh", root).unwrap()
        );
    }

    #[test]
    fn missing_invalid_and_unrepresented_imports_fail_without_fallback() {
        let source = snapshot(&[("include/a.glsl", "float a;\n")]);
        for import in [
            "<missing.glsl>",
            "<other:a.glsl>",
            "\"../../escape.glsl\"",
            "\"\"",
            "\"",
            "<>",
            "\"/a.glsl\"",
            "<a.glsl> trailing",
        ] {
            assert!(
                expand(
                    &source,
                    "minecraft",
                    "post/test.fsh",
                    &format!("#version 330\n#moj_import {import}\n")
                )
                .is_err(),
                "{import}"
            );
        }
    }

    #[test]
    fn block_comment_ending_before_import_does_not_comment_out_expansion() {
        let source = snapshot(&[("include/a.glsl", "float imported;\n")]);
        let result = expand(
            &source,
            "minecraft",
            "post/test.fsh",
            "#version 330\n/* comment\n*/ #moj_import <a.glsl>\nvoid main(){}\n",
        )
        .unwrap();
        assert!(!result.contains("/*"));
        assert!(result.contains("float imported;"));
        assert!(result.contains("void main(){}"));
        assert!(expand(&source, "minecraft", "post/test.fsh", "/* unterminated").is_err());
    }

    #[test]
    fn depth_and_expanded_bytes_are_bounded() {
        let files = (0..35)
            .map(|i| {
                ShaderSourceFile::new(
                    format!("include/{i}.glsl"),
                    format!("#moj_import <{}.glsl>\n", i + 1),
                )
            })
            .collect();
        let source = ShaderPackSource::new("depth", 1, files).unwrap();
        assert!(expand(
            &source,
            "minecraft",
            "post/test.fsh",
            "#moj_import <0.glsl>\n"
        )
        .unwrap_err()
        .message
        .contains("depth"));
        let source = ShaderPackSource::new(
            "bytes",
            1,
            vec![ShaderSourceFile::new(
                "include/a.glsl",
                " ".repeat(MAX_EXPANDED_BYTES),
            )],
        )
        .unwrap();
        assert!(expand(
            &source,
            "minecraft",
            "post/test.fsh",
            "#version 330\n#moj_import <a.glsl>\n"
        )
        .unwrap_err()
        .message
        .contains("byte bound"));
    }
}
