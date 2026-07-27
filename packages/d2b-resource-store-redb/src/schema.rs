//! Frozen ten-table physical schema.

use redb::TableDefinition;

use crate::{KeySpace, ValueKind};

pub const STORE_META: TableDefinition<'static, &[u8], &[u8]> = TableDefinition::new("store_meta");
pub const API_SCHEMAS: TableDefinition<'static, &[u8], &[u8]> = TableDefinition::new("api_schemas");
pub const RESOURCES: TableDefinition<'static, &[u8], &[u8]> = TableDefinition::new("resources");
pub const TYPE_INDEX: TableDefinition<'static, &[u8], &[u8]> = TableDefinition::new("type_index");
pub const OWNER_INDEX: TableDefinition<'static, &[u8], &[u8]> = TableDefinition::new("owner_index");
pub const PRODUCER_INDEX: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("producer_index");
pub const CONTROLLER_INDEX: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("controller_index");
pub const REVISION_LOG: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("revision_log");
pub const OPERATIONS: TableDefinition<'static, &[u8], &[u8]> = TableDefinition::new("operations");
pub const ZONE_LINK_CURSORS: TableDefinition<'static, &[u8], &[u8]> =
    TableDefinition::new("zone_link_cursors");

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
    use redb::TableHandle;

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
        assert_eq!(STORE_META.name(), "store_meta");
        assert_eq!(API_SCHEMAS.name(), "api_schemas");
        assert_eq!(RESOURCES.name(), "resources");
        assert_eq!(TYPE_INDEX.name(), "type_index");
        assert_eq!(OWNER_INDEX.name(), "owner_index");
        assert_eq!(PRODUCER_INDEX.name(), "producer_index");
        assert_eq!(CONTROLLER_INDEX.name(), "controller_index");
        assert_eq!(REVISION_LOG.name(), "revision_log");
        assert_eq!(OPERATIONS.name(), "operations");
        assert_eq!(ZONE_LINK_CURSORS.name(), "zone_link_cursors");
    }
}
