use super::*;

#[test]
fn unknown_string_becomes_custom() {
    assert!(matches!(
        "Nope".parse::<NodeType>(),
        Ok(NodeType::Custom(_))
    ));
}

#[test]
fn custom_node_without_provenance_rejected() {
    let err = validate(
        &NodeType::Custom("Wiki".to_string()),
        &EdgeType::BelongsTo,
        &Endpoint::Node(NodeType::Project),
        None,
    )
    .unwrap_err();
    assert_eq!(err, AdjacencyError::MissingProvenance);
}

#[test]
fn custom_edge_without_provenance_rejected() {
    let err = validate(
        &NodeType::Task,
        &EdgeType::Custom("mentions".to_string()),
        &Endpoint::Node(NodeType::Decision),
        None,
    )
    .unwrap_err();
    assert_eq!(err, AdjacencyError::MissingProvenance);
}

#[test]
fn adjacency_accepts_core() {
    assert!(validate(
        &NodeType::Task,
        &EdgeType::FollowsUp,
        &Endpoint::Node(NodeType::Decision),
        None,
    )
    .is_ok());
}

#[test]
fn adjacency_rejects_mismatch() {
    let err = validate(
        &NodeType::Task,
        &EdgeType::Evaluates,
        &Endpoint::Node(NodeType::Project),
        None,
    )
    .unwrap_err();
    assert_eq!(
        err,
        AdjacencyError::IllegalPair {
            from: "Task".to_string(),
            edge: "EVALUATES".to_string(),
            to: "Project".to_string(),
        }
    );
}

#[test]
fn bad_uri_rejected() {
    let err = validate(
        &NodeType::Dataset,
        &EdgeType::LocatedAt,
        &Endpoint::Uri(String::new()),
        None,
    )
    .unwrap_err();
    assert_eq!(err, AdjacencyError::BadUri(String::new()));
    let err = validate(
        &NodeType::Dataset,
        &EdgeType::LocatedAt,
        &Endpoint::Uri("relative/path".to_string()),
        None,
    )
    .unwrap_err();
    assert_eq!(err, AdjacencyError::BadUri("relative/path".to_string()));
}

#[test]
fn uri_accepted_for_located() {
    assert!(validate(
        &NodeType::Dataset,
        &EdgeType::LocatedAt,
        &Endpoint::Uri("/x".to_string()),
        None,
    )
    .is_ok());
    assert!(validate(
        &NodeType::Dataset,
        &EdgeType::LocatedAt,
        &Endpoint::Uri("s3://b/k".to_string()),
        None,
    )
    .is_ok());
}

#[test]
fn custom_collides_with_core_normalizes() {
    let custom = NodeType::Custom("task".to_string());
    let reparsed: NodeType = custom.to_string().parse().unwrap();
    assert_eq!(reparsed, NodeType::Task);
}

#[test]
fn custom_bypass_accept() {
    assert!(validate(
        &NodeType::Task,
        &EdgeType::Custom("mentions".to_string()),
        &Endpoint::Node(NodeType::Decision),
        Some("test"),
    )
    .is_ok());
}

#[test]
fn whitespace_provenance_rejected() {
    let err = validate(
        &NodeType::Custom("Wiki".to_string()),
        &EdgeType::BelongsTo,
        &Endpoint::Node(NodeType::Project),
        Some("   "),
    )
    .unwrap_err();
    assert_eq!(err, AdjacencyError::MissingProvenance);
}

#[test]
fn case_insensitive_parse() {
    for s in ["task", "TASK", "Task"] {
        assert_eq!(s.parse::<NodeType>(), Ok(NodeType::Task));
    }
}

#[test]
fn uri_under_wrong_edge_rejected() {
    let err = validate(
        &NodeType::Task,
        &EdgeType::FollowsUp,
        &Endpoint::Uri("/x".to_string()),
        None,
    )
    .unwrap_err();
    assert_eq!(
        err,
        AdjacencyError::IllegalPair {
            from: "Task".to_string(),
            edge: "FOLLOWS_UP".to_string(),
            to: "/x".to_string(),
        }
    );
}

#[test]
fn adjacency_table_full_coverage() {
    let rows: Vec<(NodeType, EdgeType, Endpoint)> = vec![
        (
            NodeType::Conversation,
            EdgeType::BelongsTo,
            Endpoint::Node(NodeType::Project),
        ),
        (
            NodeType::Conversation,
            EdgeType::FollowsUp,
            Endpoint::Node(NodeType::Conversation),
        ),
        (
            NodeType::Conversation,
            EdgeType::FollowsUp,
            Endpoint::Node(NodeType::Task),
        ),
        (
            NodeType::Task,
            EdgeType::BelongsTo,
            Endpoint::Node(NodeType::Project),
        ),
        (
            NodeType::Task,
            EdgeType::FollowsUp,
            Endpoint::Node(NodeType::Task),
        ),
        (
            NodeType::Task,
            EdgeType::FollowsUp,
            Endpoint::Node(NodeType::Decision),
        ),
        (
            NodeType::Task,
            EdgeType::FollowsUp,
            Endpoint::Node(NodeType::Conversation),
        ),
        (
            NodeType::Task,
            EdgeType::FollowsUp,
            Endpoint::Node(NodeType::Artifact),
        ),
        (
            NodeType::Decision,
            EdgeType::BelongsTo,
            Endpoint::Node(NodeType::Project),
        ),
        (
            NodeType::Artifact,
            EdgeType::BelongsTo,
            Endpoint::Node(NodeType::Project),
        ),
        (
            NodeType::Decision,
            EdgeType::FollowsUp,
            Endpoint::Node(NodeType::Task),
        ),
        (
            NodeType::Decision,
            EdgeType::FollowsUp,
            Endpoint::Node(NodeType::Decision),
        ),
        (
            NodeType::Decision,
            EdgeType::Supports,
            Endpoint::Node(NodeType::Task),
        ),
        (
            NodeType::Decision,
            EdgeType::Supports,
            Endpoint::Node(NodeType::Decision),
        ),
        (
            NodeType::Experiment,
            EdgeType::BelongsTo,
            Endpoint::Node(NodeType::Project),
        ),
        (
            NodeType::Experiment,
            EdgeType::Evaluates,
            Endpoint::Node(NodeType::Product),
        ),
        (
            NodeType::Experiment,
            EdgeType::Evaluates,
            Endpoint::Node(NodeType::Dataset),
        ),
        (
            NodeType::Experiment,
            EdgeType::Evaluates,
            Endpoint::Node(NodeType::Model),
        ),
        (
            NodeType::Experiment,
            EdgeType::Supports,
            Endpoint::Node(NodeType::Task),
        ),
        (
            NodeType::Experiment,
            EdgeType::Supports,
            Endpoint::Node(NodeType::Decision),
        ),
        (
            NodeType::Experiment,
            EdgeType::DiscussedIn,
            Endpoint::Node(NodeType::Conversation),
        ),
        (
            NodeType::Decision,
            EdgeType::DiscussedIn,
            Endpoint::Node(NodeType::Conversation),
        ),
        (
            NodeType::Model,
            EdgeType::DerivedFrom,
            Endpoint::Node(NodeType::Dataset),
        ),
        (
            NodeType::Model,
            EdgeType::DerivedFrom,
            Endpoint::Node(NodeType::Model),
        ),
        (
            NodeType::Model,
            EdgeType::DerivedFrom,
            Endpoint::Node(NodeType::Conversation),
        ),
        (
            NodeType::Dataset,
            EdgeType::DerivedFrom,
            Endpoint::Node(NodeType::Dataset),
        ),
        (
            NodeType::Dataset,
            EdgeType::DerivedFrom,
            Endpoint::Node(NodeType::Model),
        ),
        (
            NodeType::Dataset,
            EdgeType::DerivedFrom,
            Endpoint::Node(NodeType::Conversation),
        ),
        (
            NodeType::Artifact,
            EdgeType::DerivedFrom,
            Endpoint::Node(NodeType::Dataset),
        ),
        (
            NodeType::Artifact,
            EdgeType::DerivedFrom,
            Endpoint::Node(NodeType::Model),
        ),
        (
            NodeType::Artifact,
            EdgeType::DerivedFrom,
            Endpoint::Node(NodeType::Conversation),
        ),
        (
            NodeType::Artifact,
            EdgeType::Supersedes,
            Endpoint::Node(NodeType::Artifact),
        ),
        (
            NodeType::Decision,
            EdgeType::Supersedes,
            Endpoint::Node(NodeType::Decision),
        ),
        (
            NodeType::Dataset,
            EdgeType::LocatedAt,
            Endpoint::Uri("/x".to_string()),
        ),
        (
            NodeType::Model,
            EdgeType::LocatedAt,
            Endpoint::Uri("/x".to_string()),
        ),
        (
            NodeType::Artifact,
            EdgeType::LocatedAt,
            Endpoint::Uri("/x".to_string()),
        ),
        (
            NodeType::Dataset,
            EdgeType::OriginatedAt,
            Endpoint::Uri("/x".to_string()),
        ),
        (
            NodeType::Model,
            EdgeType::OriginatedAt,
            Endpoint::Uri("/x".to_string()),
        ),
        (
            NodeType::Artifact,
            EdgeType::OriginatedAt,
            Endpoint::Uri("/x".to_string()),
        ),
        (
            NodeType::Artifact,
            EdgeType::Contains,
            Endpoint::Node(NodeType::Artifact),
        ),
        (
            NodeType::Artifact,
            EdgeType::PartOf,
            Endpoint::Node(NodeType::Artifact),
        ),
        (
            NodeType::Product,
            EdgeType::Informs,
            Endpoint::Node(NodeType::Project),
        ),
        (
            NodeType::Experiment,
            EdgeType::Informs,
            Endpoint::Node(NodeType::Project),
        ),
    ];
    assert_eq!(rows.len(), 43, "plan lists exactly 43 triples");
    for (from, edge, to) in &rows {
        assert!(
            validate(from, edge, to, Some("test")).is_ok(),
            "should accept {from} -[{edge}]-> {to}"
        );
    }
    let nodes = [
        NodeType::Conversation,
        NodeType::Task,
        NodeType::Decision,
        NodeType::Experiment,
        NodeType::Dataset,
        NodeType::Model,
        NodeType::Artifact,
        NodeType::Project,
        NodeType::Product,
    ];
    let edges = [
        EdgeType::BelongsTo,
        EdgeType::FollowsUp,
        EdgeType::Supports,
        EdgeType::Evaluates,
        EdgeType::DiscussedIn,
        EdgeType::DerivedFrom,
        EdgeType::Supersedes,
        EdgeType::LocatedAt,
        EdgeType::OriginatedAt,
        EdgeType::Contains,
        EdgeType::PartOf,
        EdgeType::Informs,
    ];
    let uri = Endpoint::Uri("/x".to_string());
    let mut accepted = 0;
    for from in &nodes {
        for edge in &edges {
            for to in &nodes {
                let to_ep = Endpoint::Node(to.clone());
                if validate(from, edge, &to_ep, None).is_ok() {
                    accepted += 1;
                }
            }
            if validate(from, edge, &uri, None).is_ok() {
                accepted += 1;
            }
        }
    }
    assert_eq!(accepted, 43, "core table must accept exactly 43 triples");
}

#[test]
fn materialize_applies_upserts_and_retracts() {
    let events = vec![
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z","label":"a"}}),
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-02T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-02T00:00:00Z","label":"b"}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-03T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"FOLLOWS_UP","valid_from":"2026-01-03T00:00:00Z","valid_until":null}}),
        serde_json::json!({"op":"edge.retract","observed_utc":"2026-01-04T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"FOLLOWS_UP"}}),
    ];
    let m = materialize(&events, Some("2026-06-01T00:00:00Z"), false);
    assert_eq!(m.nodes.len(), 2);
    assert_eq!(m.edges.len(), 1);
    assert!(m.edges[0].retracted);
}

#[test]
fn as_of_boundary_inclusive() {
    let as_of = "2026-06-01T00:00:00Z";
    let events = vec![
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":"2026-06-01T00:00:00Z"}}),
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:3","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-06-01T00:00:00Z","payload":{"from":"n:2","to":"n:3","type":"SUPPORTS","valid_from":"2026-06-01T00:00:00Z","valid_until":null}}),
    ];
    let m = materialize(&events, Some(as_of), false);
    assert_eq!(m.edges.len(), 2);
}

#[test]
fn future_valid_from_excluded_and_null_never_expires() {
    let events = vec![
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2026-07-01T00:00:00Z","valid_until":null}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:2","to":"n:1","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":null}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:1","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z"}}),
    ];
    let m = materialize(&events, Some("2026-06-01T00:00:00Z"), false);
    assert_eq!(m.edges.len(), 2);
}

#[test]
fn observed_utc_max_merge() {
    let events = vec![
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-02T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-02T00:00:00Z","label":"old"}}),
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z","label":"new"}}),
    ];
    let m = materialize(&events, Some("2026-06-01T00:00:00Z"), false);
    let node = m.nodes.get("n:1").expect("n:1 present");
    assert_eq!(node.get("label").and_then(|v| v.as_str()), Some("new"));
    assert_eq!(
        node.get("observed_utc").and_then(|v| v.as_str()),
        Some("2026-01-02T00:00:00Z")
    );
}

#[test]
fn include_expired_bypasses_validity_filter() {
    let as_of = "2026-06-01T00:00:00Z";
    let events = vec![
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":"2026-02-01T00:00:00Z"}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:2","to":"n:1","type":"SUPPORTS","valid_from":"2026-07-01T00:00:00Z","valid_until":null}}),
    ];
    let filtered = materialize(&events, Some(as_of), false);
    assert_eq!(filtered.edges.len(), 0);
    let unfiltered = materialize(&events, Some(as_of), true);
    assert_eq!(unfiltered.edges.len(), 2);
}

#[test]
fn replay_skips_malformed_and_unknown_ops() {
    let events = vec![
        serde_json::json!({"foo": 1}),
        serde_json::json!({"op": "node.upsert"}),
        serde_json::json!({"op": "node.upsert", "payload": "not-an-object"}),
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"label": "no-id"}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from": "n:1"}}),
        serde_json::json!({"op":"config.set","observed_utc":"2026-01-01T00:00:00Z","payload":{"key": "theme", "value": "dark"}}),
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":123}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:2","to":"n:1","type":"SUPPORTS","valid_until":null}}),
    ];
    let m = materialize(&events, Some("2026-06-01T00:00:00Z"), false);
    assert_eq!(m.nodes.len(), 2);
    assert_eq!(m.edges.len(), 2);
    assert!(m.edges.iter().all(|e| e.valid_until.is_none()));
    let defaulted = m.edges.iter().find(|e| e.from == "n:2").unwrap();
    assert_eq!(defaulted.valid_from, "2026-01-02T00:00:00Z");
}

#[test]
fn retract_specificity() {
    let events = vec![
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:3","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":null}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"FOLLOWS_UP","valid_from":"2026-01-01T00:00:00Z","valid_until":null}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:3","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":null}}),
        serde_json::json!({"op":"edge.retract","observed_utc":"2026-01-03T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS"}}),
    ];
    let m = materialize(&events, Some("2026-06-01T00:00:00Z"), true);
    assert_eq!(m.edges.len(), 3);
    for e in &m.edges {
        let is_target = e.from == "n:1" && e.to == "n:2" && e.edge == EdgeType::Supports;
        assert_eq!(
            e.retracted, is_target,
            "only exact (from, edge, to) retracts"
        );
    }
}

#[test]
fn nodes_never_filtered() {
    let events = vec![
        serde_json::json!({"op":"node.upsert","observed_utc":"2020-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2020-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"node.upsert","observed_utc":"2020-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2020-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2020-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2020-01-01T00:00:00Z","valid_until":"2020-02-01T00:00:00Z"}}),
    ];
    let m = materialize(&events, Some("2026-06-01T00:00:00Z"), false);
    assert_eq!(m.edges.len(), 0);
    assert_eq!(m.nodes.len(), 2);
    assert!(m.nodes.contains_key("n:1"));
    assert!(m.nodes.contains_key("n:2"));
}

#[test]
fn offset_timestamps_skipped() {
    let events = vec![
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:1","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"node.upsert","observed_utc":"2026-01-01T00:00:00Z","payload":{"id":"n:2","observed_utc":"2026-01-01T00:00:00Z"}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00+08:00","valid_until":null}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:2","type":"FOLLOWS_UP","valid_from":"2026-01-01T00:00:00.123Z","valid_until":null}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:2","to":"n:1","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":"2026-12-31T00:00:00+08:00"}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00+08:00","payload":{"from":"n:2","to":"n:1","type":"FOLLOWS_UP","valid_from":"2026-01-01T00:00:00Z","valid_until":null}}),
        serde_json::json!({"op":"edge.assert","observed_utc":"2026-01-02T00:00:00Z","payload":{"from":"n:1","to":"n:1","type":"SUPPORTS","valid_from":"2026-01-01T00:00:00Z","valid_until":null}}),
    ];
    let as_of = Some("2026-06-01T00:00:00Z");
    let filtered = materialize(&events, as_of, false);
    assert_eq!(filtered.edges.len(), 1);
    assert_eq!(filtered.edges[0].from, "n:1");
    assert_eq!(filtered.edges[0].to, "n:1");
    let unfiltered = materialize(&events, as_of, true);
    assert_eq!(unfiltered.edges.len(), 1);
}
