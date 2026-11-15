//! `forms.renderers`: how form/widget templates are found and rendered.

use std::sync::{Arc, LazyLock};

use rango_template::{Context, Engine, TemplateError, Value as T};

use crate::templates_data::TEMPLATES;

/// A form renderer: `render(template_name, context)` -> HTML.
pub trait Renderer: Send + Sync {
    fn render(&self, template_name: &str, context: T) -> Result<String, TemplateError>;
    fn form_template_name(&self) -> &str {
        "rango/forms/div.html"
    }
    fn formset_template_name(&self) -> &str {
        "rango/forms/formsets/div.html"
    }
    fn field_template_name(&self) -> &str {
        "rango/forms/field.html"
    }
}

fn context_from(value: T) -> Context {
    let mut c = Context::new();
    if let T::Map(m) = value {
        for (k, v) in m {
            c.set(&k, v);
        }
    }
    c
}

static ENGINE: LazyLock<Engine> = LazyLock::new(|| {
    let mut e = Engine::new();
    e.locmem(TEMPLATES);
    e
});

/// `renderers.RangoTemplates`: the built-in form templates only.
#[derive(Debug, Clone, Copy, Default)]
pub struct RangoTemplates;

impl Renderer for RangoTemplates {
    fn render(&self, template_name: &str, context: T) -> Result<String, TemplateError> {
        let mut ctx = context_from(context);
        Ok(ENGINE.get_template(template_name)?.render(&ENGINE, &mut ctx)?.trim().to_owned())
    }
}

/// `renderers.TemplatesSetting`: look templates up with the project's `TEMPLATES` (so they can be overridden).
#[derive(Debug, Clone, Copy, Default)]
pub struct TemplatesSetting;

impl Renderer for TemplatesSetting {
    fn render(&self, template_name: &str, context: T) -> Result<String, TemplateError> {
        let engine = rango_template::loader::engine()?;
        let mut ctx = context_from(context);
        Ok(engine.get_template(template_name).or_else(|_| ENGINE.get_template(template_name))?.render(&engine, &mut ctx)?.trim().to_owned())
    }
}

/// `get_default_renderer()` from `settings.FORM_RENDERER`.
pub fn get_default_renderer() -> Arc<dyn Renderer> {
    let name = rango_core::conf::try_settings().map(|s| s.form_renderer.clone()).unwrap_or_default();
    if name.ends_with("TemplatesSetting") { Arc::new(TemplatesSetting) } else { Arc::new(RangoTemplates) }
}
