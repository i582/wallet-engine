use std::collections::BTreeSet;

use anyhow::{Context, Result, ensure};
use uniffi_bindgen::interface::{AsType, Enum, Type, Variant};

use crate::{
    naming,
    type_registry::{NestedWireSize, RegisteredType, TypeRegistry},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TaggedEnumType {
    uniffi_type: Type,
    rust_name: String,
    c_name: String,
    tag_c_name: String,
    payload_c_name: String,
    function_name: String,
    variants: Vec<TaggedEnumVariant>,
    read_needs_arena: bool,
}

impl TaggedEnumType {
    pub(super) fn rust_name(&self) -> &str {
        &self.rust_name
    }

    pub(super) fn c_name(&self) -> &str {
        &self.c_name
    }

    pub(super) fn tag_c_name(&self) -> &str {
        &self.tag_c_name
    }

    pub(super) fn payload_c_name(&self) -> &str {
        &self.payload_c_name
    }

    pub(super) fn function_name(&self) -> &str {
        &self.function_name
    }

    pub(super) fn variants(&self) -> &[TaggedEnumVariant] {
        &self.variants
    }

    pub(super) const fn read_needs_arena(&self) -> bool {
        self.read_needs_arena
    }

    fn registered_type(&self) -> RegisteredType {
        RegisteredType::compound(
            self.rust_name.clone(),
            self.c_name.clone(),
            self.rust_name.clone(),
            self.function_name.clone(),
            4,
            self.read_needs_arena,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TaggedEnumVariant {
    rust_name: String,
    c_constant: String,
    public_value: u32,
    wire_tag: i32,
    payload_c_name: Option<String>,
    payload_member_name: String,
    fields: Vec<TaggedEnumField>,
}

impl TaggedEnumVariant {
    pub(super) fn rust_name(&self) -> &str {
        &self.rust_name
    }

    pub(super) fn c_constant(&self) -> &str {
        &self.c_constant
    }

    pub(super) const fn public_value(&self) -> u32 {
        self.public_value
    }

    pub(super) const fn wire_tag(&self) -> i32 {
        self.wire_tag
    }

    pub(super) fn payload_c_name(&self) -> Option<&str> {
        self.payload_c_name.as_deref()
    }

    pub(super) fn payload_member_name(&self) -> &str {
        &self.payload_member_name
    }

    pub(super) fn fields(&self) -> &[TaggedEnumField] {
        &self.fields
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TaggedEnumField {
    rust_name: String,
    c_name: String,
    rust_type_name: String,
    c_type_name: String,
    codec_name: String,
    nested_wire_size: NestedWireSize,
    read_needs_arena: bool,
}

impl TaggedEnumField {
    pub(super) fn rust_name(&self) -> &str {
        &self.rust_name
    }

    pub(super) fn c_name(&self) -> &str {
        &self.c_name
    }

    pub(super) fn rust_type_name(&self) -> &str {
        &self.rust_type_name
    }

    pub(super) fn c_type_name(&self) -> &str {
        &self.c_type_name
    }

    pub(super) fn codec_name(&self) -> &str {
        &self.codec_name
    }

    pub(super) const fn nested_wire_size(&self) -> NestedWireSize {
        self.nested_wire_size
    }

    pub(super) const fn read_needs_arena(&self) -> bool {
        self.read_needs_arena
    }
}

pub(super) fn collect_tagged_enum_types(
    mut remaining: Vec<&Enum>,
    types: &mut TypeRegistry,
    kind: &str,
) -> Result<Vec<TaggedEnumType>> {
    remaining.sort_by(|left, right| left.name().cmp(right.name()));

    let mut enums = Vec::new();
    loop {
        let previous_len = remaining.len();
        let mut pending = Vec::new();
        for enum_ in remaining {
            let Some(tagged) = tagged_enum_type(enum_, types, kind)? else {
                pending.push(enum_);
                continue;
            };
            reserve_auxiliary_c_names(&tagged, types)?;
            types.register_type(&tagged.uniffi_type, tagged.registered_type())?;
            enums.push(tagged);
        }
        if pending.is_empty() || pending.len() == previous_len {
            return Ok(enums);
        }
        remaining = pending;
    }
}

fn tagged_enum_type(
    enum_: &Enum,
    types: &TypeRegistry,
    kind: &str,
) -> Result<Option<TaggedEnumType>> {
    let rust_name = enum_.name().to_owned();
    let mut constants = BTreeSet::new();
    let mut payload_members = BTreeSet::new();
    let variants = enum_
        .variants()
        .iter()
        .enumerate()
        .map(|(variant_index, variant)| {
            tagged_enum_variant(
                &rust_name,
                variant_index,
                variant,
                types,
                kind,
                &mut constants,
                &mut payload_members,
            )
        })
        .collect::<Result<Option<Vec<_>>>>()?;
    let Some(variants) = variants else {
        return Ok(None);
    };
    ensure!(!variants.is_empty(), "{kind} {rust_name} has no variants");
    ensure!(
        variants.iter().any(|variant| !variant.fields.is_empty()),
        "{kind} {rust_name} has no payload variants"
    );
    let read_needs_arena = variants
        .iter()
        .flat_map(|variant| &variant.fields)
        .any(TaggedEnumField::read_needs_arena);

    Ok(Some(TaggedEnumType {
        uniffi_type: enum_.as_type(),
        c_name: naming::type_name(&rust_name),
        tag_c_name: naming::type_name(&format!("{rust_name}Tag")),
        payload_c_name: naming::type_name(&format!("{rust_name}Payload")),
        function_name: naming::function_name(&rust_name),
        rust_name,
        variants,
        read_needs_arena,
    }))
}

fn tagged_enum_variant(
    enum_rust_name: &str,
    variant_index: usize,
    variant: &Variant,
    types: &TypeRegistry,
    kind: &str,
    constants: &mut BTreeSet<String>,
    payload_members: &mut BTreeSet<String>,
) -> Result<Option<TaggedEnumVariant>> {
    let public_value = u32::try_from(variant_index).with_context(|| {
        format!("{kind} {enum_rust_name} has too many variants for the public C ABI")
    })?;
    let wire_tag = i32::try_from(variant_index.saturating_add(1)).with_context(|| {
        format!("{kind} {enum_rust_name} has too many variants for the UniFFI wire ABI")
    })?;
    let c_constant = naming::constant_name(enum_rust_name, variant.name());
    ensure!(
        constants.insert(c_constant.clone()),
        "{kind} {enum_rust_name} has C constant collision at {c_constant}"
    );

    let mut c_field_names = BTreeSet::new();
    let fields = variant
        .fields()
        .iter()
        .enumerate()
        .map(|(field_index, field)| {
            let field_type = field.as_type();
            let registered = types.resolve(&field_type)?;
            let rust_field_name = field.name().to_owned();
            let c_name = if rust_field_name.is_empty() {
                format!("field_{field_index}")
            } else {
                naming::field_name(&rust_field_name)
            };
            Some((registered, rust_field_name, c_name))
        })
        .collect::<Option<Vec<_>>>();
    let Some(fields) = fields else {
        return Ok(None);
    };
    let fields = fields
        .into_iter()
        .map(|(registered, rust_field_name, c_name)| {
            ensure!(
                c_field_names.insert(c_name.clone()),
                "{kind} {enum_rust_name} variant {} produces duplicate C field {c_name}",
                variant.name()
            );
            Ok(TaggedEnumField {
                rust_name: rust_field_name,
                c_name,
                rust_type_name: registered.rust_name().to_owned(),
                c_type_name: registered.c_name().to_owned(),
                codec_name: registered.codec_name().to_owned(),
                nested_wire_size: registered.nested_wire_size(),
                read_needs_arena: registered.read_needs_arena(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let payload_member_name = naming::field_name(variant.name());
    let payload_c_name = if fields.is_empty() {
        None
    } else {
        ensure!(
            payload_members.insert(payload_member_name.clone()),
            "{kind} {enum_rust_name} produces duplicate C payload member {payload_member_name}"
        );
        Some(naming::type_name(&format!(
            "{enum_rust_name}{}Payload",
            variant.name()
        )))
    };

    Ok(Some(TaggedEnumVariant {
        rust_name: variant.name().to_owned(),
        c_constant,
        public_value,
        wire_tag,
        payload_c_name,
        payload_member_name,
        fields,
    }))
}

fn reserve_auxiliary_c_names(enum_: &TaggedEnumType, types: &mut TypeRegistry) -> Result<()> {
    types.reserve_c_name(enum_.tag_c_name())?;
    types.reserve_c_name(enum_.payload_c_name())?;
    for variant in enum_.variants() {
        if let Some(payload_c_name) = variant.payload_c_name() {
            types.reserve_c_name(payload_c_name)?;
        }
    }
    Ok(())
}
