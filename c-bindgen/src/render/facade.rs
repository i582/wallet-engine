use crate::{model::BindingsModel, template};

const FACADE_TEMPLATE: &str = include_str!("../../templates/facade.c.tmpl");
const WIRE_INCLUDES_TEMPLATE: &str = include_str!("../../templates/wire_includes.c.tmpl");
const ARENA_INCLUDES_TEMPLATE: &str = include_str!("../../templates/arena_includes.c.tmpl");

pub(super) fn render(model: &BindingsModel) -> String {
    let wire_includes = if model.has_wire_types() {
        let mut includes = String::from(WIRE_INCLUDES_TEMPLATE);
        if model.needs_output_arena() {
            includes.push_str(ARENA_INCLUDES_TEMPLATE);
        }
        includes
    } else {
        String::new()
    };
    let mut facade = template::render(FACADE_TEMPLATE, &[("WIRE_INCLUDES", &wire_includes)]);
    facade.push_str(&super::codec::render(model));
    facade
}

#[cfg(test)]
mod tests {
    use super::render;

    #[test]
    fn facade_includes_its_generated_header() -> anyhow::Result<()> {
        let model = crate::model::BindingsModel::from_components(&[
            uniffi_bindgen::ComponentInterface::new("wallet_engine"),
        ])?;
        let facade = render(&model);

        assert!(facade.contains("#include \"wallet_engine.h\""));
        assert!(facade.contains("uint32_t wallet_engine_abi_version(void)"));
        assert!(!facade.contains("#include <string.h>"));
        Ok(())
    }
}
