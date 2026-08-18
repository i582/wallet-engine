use std::{collections::BTreeMap, fmt::Write as _};

use anyhow::{Result, bail};
use uniffi_bindgen::{
    ComponentInterface,
    interface::{AsType, Enum, Record, Type},
};

use crate::type_registry::TypeRegistry;

pub(super) fn validate_reachable_type_coverage(
    component: &ComponentInterface,
    types: &TypeRegistry,
) -> Result<()> {
    let missing_records = component
        .record_definitions()
        .iter()
        .filter(|record| {
            is_local_record(record, component) && types.resolve(&record.as_type()).is_none()
        })
        .map(|record| {
            (
                record.name().to_owned(),
                unresolved_record_fields(record, types),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let missing_enums = component.enum_definitions().iter().filter(|enum_| {
        is_local_enum(enum_, component) && types.resolve(&enum_.as_type()).is_none()
    });
    let mut missing_fielded_enums = BTreeMap::new();
    let mut missing_errors = BTreeMap::new();
    for enum_ in missing_enums {
        let details = unresolved_enum_fields(enum_, types);
        if component.is_name_used_as_error(enum_.name()) {
            missing_errors.insert(enum_.name().to_owned(), details);
        } else {
            missing_fielded_enums.insert(enum_.name().to_owned(), details);
        }
    }

    if missing_records.is_empty() && missing_fielded_enums.is_empty() && missing_errors.is_empty() {
        return Ok(());
    }

    let mut diagnostic =
        String::from("unsupported reachable UniFFI types after the final C type closure:");
    append_missing(&mut diagnostic, "records", &missing_records);
    append_missing(&mut diagnostic, "fielded enums", &missing_fielded_enums);
    append_missing(&mut diagnostic, "declared errors", &missing_errors);
    bail!(diagnostic)
}

fn is_local_record(record: &Record, component: &ComponentInterface) -> bool {
    !record.remote() && is_local_named_type(&record.as_type(), component.crate_name())
}

fn is_local_enum(enum_: &Enum, component: &ComponentInterface) -> bool {
    !enum_.remote() && is_local_named_type(&enum_.as_type(), component.crate_name())
}

fn is_local_named_type(type_: &Type, crate_name: &str) -> bool {
    match type_ {
        Type::Record { module_path, .. } | Type::Enum { module_path, .. } => {
            module_path.split("::").next() == Some(crate_name)
        }
        _ => false,
    }
}

fn unresolved_record_fields(record: &Record, types: &TypeRegistry) -> Vec<String> {
    let fields = record
        .fields()
        .iter()
        .filter_map(|field| {
            let field_type = field.as_type();
            types
                .resolve(&field_type)
                .is_none()
                .then(|| format!("{}: {}", field.name(), type_label(&field_type)))
        })
        .collect::<Vec<_>>();
    fallback_details(fields, "record was not registered")
}

fn unresolved_enum_fields(enum_: &Enum, types: &TypeRegistry) -> Vec<String> {
    let fields = enum_
        .variants()
        .iter()
        .flat_map(|variant| {
            variant
                .fields()
                .iter()
                .enumerate()
                .filter_map(move |(index, field)| {
                    let field_type = field.as_type();
                    types.resolve(&field_type).is_none().then(|| {
                        let field_name = if field.name().is_empty() {
                            format!("field_{index}")
                        } else {
                            field.name().to_owned()
                        };
                        format!(
                            "{}.{}: {}",
                            variant.name(),
                            field_name,
                            type_label(&field_type)
                        )
                    })
                })
        })
        .collect::<Vec<_>>();
    let fallback = if enum_.is_flat() {
        "flat declared error representation is not implemented"
    } else {
        "tagged enum was not registered after its dependencies became available"
    };
    fallback_details(fields, fallback)
}

fn fallback_details(mut details: Vec<String>, fallback: &str) -> Vec<String> {
    if details.is_empty() {
        details.push(fallback.to_owned());
    }
    details
}

fn type_label(type_: &Type) -> String {
    match type_ {
        Type::UInt8 => "u8".to_owned(),
        Type::Int8 => "i8".to_owned(),
        Type::UInt16 => "u16".to_owned(),
        Type::Int16 => "i16".to_owned(),
        Type::UInt32 => "u32".to_owned(),
        Type::Int32 => "i32".to_owned(),
        Type::UInt64 => "u64".to_owned(),
        Type::Int64 => "i64".to_owned(),
        Type::Float32 => "f32".to_owned(),
        Type::Float64 => "f64".to_owned(),
        Type::Boolean => "bool".to_owned(),
        Type::String => "String".to_owned(),
        Type::Bytes => "Vec<u8>".to_owned(),
        Type::Record { name, .. } | Type::Enum { name, .. } | Type::Custom { name, .. } => {
            name.clone()
        }
        Type::Optional { inner_type } => format!("Option<{}>", type_label(inner_type)),
        Type::Sequence { inner_type } => format!("Vec<{}>", type_label(inner_type)),
        _ => format!("{type_:?}"),
    }
}

fn append_missing(
    diagnostic: &mut String,
    category: &str,
    missing: &BTreeMap<String, Vec<String>>,
) {
    if missing.is_empty() {
        return;
    }
    let _ = write!(diagnostic, "\n  {category}:");
    for (name, details) in missing {
        let _ = write!(diagnostic, "\n    - {name}: {}", details.join(", "));
    }
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use uniffi_bindgen::ComponentInterface;

    use crate::model::BindingsModel;

    #[test]
    fn reports_every_unresolved_named_type_with_actionable_details() -> Result<()> {
        let component = ComponentInterface::from_webidl(
            r#"
            namespace wallet_engine {
                [Throws=FlatFailure]
                void fail();
            };

            dictionary Detail { string value; };
            dictionary Request { Payload payload; };

            [Enum]
            interface Payload { Value(Detail detail); };

            [Error]
            enum FlatFailure { "failed" };
            "#,
            "wallet_engine",
        )?;
        let error = BindingsModel::from_components(&[component])
            .expect_err("unresolved types should fail final coverage")
            .to_string();

        assert!(error.contains("records:\n    - Request: payload: Payload"));
        assert!(error.contains(
            "fielded enums:\n    - Payload: tagged enum was not registered after its dependencies became available"
        ));
        assert!(error.contains(
            "declared errors:\n    - FlatFailure: flat declared error representation is not implemented"
        ));
        Ok(())
    }
}
