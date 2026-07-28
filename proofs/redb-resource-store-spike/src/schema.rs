use redb::TableDefinition;

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

pub type PhysicalTableDefinition = TableDefinition<'static, &'static [u8], &'static [u8]>;
pub type PhysicalTable = (PhysicalTableDefinition, u8, u16);

pub const TABLES: [PhysicalTable; 10] = [
    (STORE_META, 0x01, 0x0001),
    (API_SCHEMAS, 0x02, 0x0002),
    (RESOURCES, 0x03, 0x0003),
    (TYPE_INDEX, 0x04, 0x0004),
    (OWNER_INDEX, 0x05, 0x0005),
    (PRODUCER_INDEX, 0x06, 0x0006),
    (CONTROLLER_INDEX, 0x07, 0x0007),
    (REVISION_LOG, 0x08, 0x0008),
    (OPERATIONS, 0x09, 0x0009),
    (ZONE_LINK_CURSORS, 0x0a, 0x000a),
];

#[cfg(test)]
mod tests {
    use super::*;
    use redb::TableHandle;

    #[test]
    fn table_names_and_discriminants_are_exact_and_contiguous() {
        let expected = [
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
        ];
        assert_eq!(TABLES.len(), 10);
        for (index, ((table, key_space, value_kind), name)) in
            TABLES.iter().zip(expected).enumerate()
        {
            assert_eq!(table.name(), name);
            assert_eq!(*key_space, u8::try_from(index + 1).unwrap());
            assert_eq!(*value_kind, u16::try_from(index + 1).unwrap());
        }
    }
}
