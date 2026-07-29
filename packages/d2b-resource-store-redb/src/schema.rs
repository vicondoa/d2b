//! Frozen ten-table physical schema.

use crate::{KeySpace, ValueKind};

/// One permanent table/discriminant assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableSchema {
    pub name: &'static str,
    pub key_space: KeySpace,
    pub value_kind: ValueKind,
}

/// Exact permanent table order and discriminant assignments.
pub const TABLE_SCHEMAS: [TableSchema; 10] = [
    TableSchema {
        name: "store_meta",
        key_space: KeySpace::StoreMeta,
        value_kind: ValueKind::StoreMetaScalar,
    },
    TableSchema {
        name: "api_schemas",
        key_space: KeySpace::ApiSchemas,
        value_kind: ValueKind::ApiSchemaRecord,
    },
    TableSchema {
        name: "resources",
        key_space: KeySpace::Resources,
        value_kind: ValueKind::ResourceRecord,
    },
    TableSchema {
        name: "type_index",
        key_space: KeySpace::TypeIndex,
        value_kind: ValueKind::TypeIndexRecord,
    },
    TableSchema {
        name: "owner_index",
        key_space: KeySpace::OwnerIndex,
        value_kind: ValueKind::OwnerIndexRecord,
    },
    TableSchema {
        name: "producer_index",
        key_space: KeySpace::ProducerIndex,
        value_kind: ValueKind::ProducerIndexRecord,
    },
    TableSchema {
        name: "controller_index",
        key_space: KeySpace::ControllerIndex,
        value_kind: ValueKind::ControllerIndexRecord,
    },
    TableSchema {
        name: "revision_log",
        key_space: KeySpace::RevisionLog,
        value_kind: ValueKind::ChangeBatch,
    },
    TableSchema {
        name: "operations",
        key_space: KeySpace::Operations,
        value_kind: ValueKind::OperationRecord,
    },
    TableSchema {
        name: "zone_link_cursors",
        key_space: KeySpace::ZoneLinkCursors,
        value_kind: ValueKind::ZoneLinkCursor,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ten_table_names_and_discriminants_are_contiguous_and_literal() {
        assert_eq!(TABLE_SCHEMAS.len(), 10);
        assert_eq!(
            TABLE_SCHEMAS.map(|table| table.name),
            [
                "store_meta",
                "api_schemas",
                "resources",
                "type_index",
                "owner_index",
                "producer_index",
                "controller_index",
                "revision_log",
                "operations",
                "zone_link_cursors",
            ]
        );
        for (index, table) in TABLE_SCHEMAS.iter().enumerate() {
            let discriminant = u8::try_from(index + 1).unwrap();
            assert_eq!(table.key_space.discriminant(), discriminant);
            assert_eq!(table.value_kind.discriminant(), u16::from(discriminant));
        }
    }
}
