use serde::Serialize;
use std::marker::PhantomData;

/// Converts a backend-specific IR (e.g. a workflow YAML model or a bash script
/// model) into its final text. All text generation goes through a `Renderer`.
pub trait Renderer {
    type Ir;
    fn render(&self, ir: &Self::Ir) -> String;
}

pub struct YamlRenderer<T>(PhantomData<T>);

impl<T> Default for YamlRenderer<T> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<T: Serialize> Renderer for YamlRenderer<T> {
    type Ir = T;
    fn render(&self, ir: &T) -> String {
        serde_yaml::to_string(ir).expect("serialization failed")
    }
}

pub struct BashUnit {
    pub label: String,
    pub lines: Vec<String>,
}

pub struct BashScript {
    pub units: Vec<BashUnit>,
}

pub struct BashRenderer;

impl Renderer for BashRenderer {
    type Ir = BashScript;
    fn render(&self, script: &BashScript) -> String {
        let mut out = String::from("#!/usr/bin/env bash\nset -euo pipefail\n");
        for unit in &script.units {
            if !unit.lines.is_empty() {
                out.push('\n');
                out.push_str(&format!("# {}\n", unit.label));
                for line in &unit.lines {
                    out.push_str(line);
                    if !line.ends_with('\n') {
                        out.push('\n');
                    }
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yaml_renderer_serializes_struct() {
        #[derive(Serialize)]
        struct Foo {
            x: u32,
        }
        let r = YamlRenderer::default();
        assert_eq!(r.render(&Foo { x: 42 }).trim(), "x: 42");
    }

    #[test]
    fn bash_renderer_emits_shebang_and_sections() {
        let script = BashScript {
            units: vec![BashUnit {
                label: "build".into(),
                lines: vec!["cargo build".into()],
            }],
        };
        let out = BashRenderer.render(&script);
        assert!(out.starts_with("#!/usr/bin/env bash"));
        assert!(out.contains("# build\ncargo build"));
    }

    #[test]
    fn bash_renderer_skips_empty_units() {
        let script = BashScript {
            units: vec![BashUnit {
                label: "empty".into(),
                lines: vec![],
            }],
        };
        let out = BashRenderer.render(&script);
        assert!(!out.contains("# empty"));
    }
}
