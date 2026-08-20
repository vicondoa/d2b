use d2b_contracts_resource::resource_proto::GetRequest;

#[test]
fn resource_messages_remain_owned_by_resource_contracts() {
    let request = GetRequest::new();
    let _ = request;
}
