//! Real-PostgreSQL integration tests for Agency Service.
//!
//! Requires DATABASE_URL pointing at a migrated `agency_db`.
//! Run: DATABASE_URL=postgres://postgres:***@127.0.0.1:5432/agency_db cargo test --test db_tests
//!
//! Covers: migrations applied, CRUD, temporal hierarchy/assignments,
//! cycle prevention, overlap rejection, transfers, outbox+audit atomicity,
//! optimistic concurrency, idempotency, and concurrent reparent safety.

use std::time::Duration;

use chrono::NaiveDate;
use iaop_agency_service_application::commands::*;
use iaop_agency_service_application::error::AppError;
use iaop_agency_service_application::ports::{IdempotencyReserve, IdempotencyStore, OrgStore};
use iaop_agency_service_application::principal::Principal;
use iaop_agency_service_domain::assignment::AssignmentType;
use iaop_agency_service_domain::entity_id::EntityId;
use iaop_agency_service_domain::org_unit::UnitKind;
use iaop_agency_service_infrastructure::idempotency::PgIdempotencyStore;
use iaop_agency_service_infrastructure::org_store::PgOrgStoreWithPool;
use sqlx::PgPool;
use uuid::Uuid;

fn principal() -> Principal {
    Principal {
        subject: "test-admin".into(),
        issuer: "test".into(),
        scopes: vec![],
        roles: vec![
            "IAOP_AGENCY_ADMIN".into(),
            "agency:create".into(),
            "agency:read".into(),
            "agency:update".into(),
            "agency:hierarchy:manage".into(),
            "agency:assignment:manage".into(),
            "agency:transfer:request".into(),
            "agency:leadership:manage".into(),
        ],
        agent_scope: None,
    }
}

async fn pool() -> PgPool {
    let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
        "postgres://postgres:340ce6f29651795482a97e89a2dd36b6f9336f60ef2ed6c4@127.0.0.1:5432/agency_db".into()
    });
    let pool = PgPool::connect(&url)
        .await
        .expect("failed to connect to agency_db");
    iaop_agency_service_infrastructure::persistence::run_migrations(&pool)
        .await
        .expect("failed to run migrations");
    pool
}

fn store(pool: &PgPool) -> PgOrgStoreWithPool {
    PgOrgStoreWithPool { pool: pool.clone() }
}

fn d(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

async fn mk_agency(pool: &PgPool, code: &str) -> EntityId {
    let store = store(pool);
    let mut tx = pool.begin().await.unwrap();
    let dto = create_agency(
        &store,
        &mut tx,
        CreateAgency {
            code: code.into(),
            name: format!("Agency {code}"),
            principal: principal(),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    dto.id
}

async fn mk_unit(
    pool: &PgPool,
    agency: EntityId,
    code: &str,
    kind: UnitKind,
    parent: Option<EntityId>,
    from: &str,
) -> EntityId {
    let store = store(pool);
    let mut tx = pool.begin().await.unwrap();
    let dto = create_unit(
        &store,
        &mut tx,
        CreateUnit {
            agency_id: agency,
            code: code.into(),
            name: format!("Unit {code}"),
            kind,
            parent_id: parent,
            effective_from: d(from),
            principal: principal(),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    dto.id
}

// ---------- schema / CRUD ----------

#[tokio::test]
async fn agency_crud_and_unique_code() {
    let pool = pool().await;
    let code = format!("AG{}", &Uuid::now_v7().simple().to_string()[20..28]);
    let id = mk_agency(&pool, &code).await;
    let store = store(&pool);
    assert!(store.get_agency(id).await.unwrap().is_some());
    assert!(store.get_agency_by_code(&code).await.unwrap().is_some());

    // duplicate code rejected
    let mut tx = pool.begin().await.unwrap();
    let err = create_agency(
        &store,
        &mut tx,
        CreateAgency {
            code: code.clone(),
            name: "dup".into(),
            principal: principal(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::Conflict(_)));
    drop(tx);
}

#[tokio::test]
async fn outbox_and_audit_written_atomically() {
    let pool = pool().await;
    let code = format!("AG{}", &Uuid::now_v7().simple().to_string()[20..28]);
    let id = mk_agency(&pool, &code).await;
    let n: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox_events WHERE aggregate_id=$1 AND event_type='agency.created.v1'",
    )
    .bind(id.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(n, 1, "exactly one outbox event for agency.create");
    let a: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agency_audit_events WHERE aggregate_id=$1 AND action='agency.create'",
    )
    .bind(id.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(a, 1, "audit record written");
}

// ---------- hierarchy ----------

#[tokio::test]
async fn hierarchy_ancestors_descendants_and_cycle_rejection() {
    let pool = pool().await;
    let ag = mk_agency(
        &pool,
        &format!("AG{}", &Uuid::now_v7().simple().to_string()[20..28]),
    )
    .await;
    let region = mk_unit(&pool, ag, "R1", UnitKind::Region, None, "2026-01-01").await;
    let branch_a = mk_unit(
        &pool,
        ag,
        "BA",
        UnitKind::Branch,
        Some(region),
        "2026-01-01",
    )
    .await;
    let _branch_b = mk_unit(
        &pool,
        ag,
        "BB",
        UnitKind::Branch,
        Some(region),
        "2026-01-01",
    )
    .await;
    let team = mk_unit(
        &pool,
        ag,
        "T1",
        UnitKind::Team,
        Some(branch_a),
        "2026-01-01",
    )
    .await;

    let store = store(&pool);
    let anc = store.ancestors(team).await.unwrap();
    assert_eq!(anc.len(), 2);
    assert!(anc.contains(&branch_a) && anc.contains(&region));
    let desc = store.descendants(region).await.unwrap();
    assert_eq!(desc.len(), 3);

    // moving region under its own descendant must fail (cycle)
    let mut tx = pool.begin().await.unwrap();
    let err = change_parent(
        &store,
        &mut tx,
        ChangeParent {
            unit_id: region,
            new_parent_id: Some(team),
            effective_from: d("2026-02-01"),
            expected_version: 1,
            principal: principal(),
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, AppError::Conflict(_)),
        "cycle must be rejected: {err:?}"
    );
    drop(tx);
}

#[tokio::test]
async fn temporal_hierarchy_as_of() {
    let pool = pool().await;
    let ag = mk_agency(
        &pool,
        &format!("AG{}", &Uuid::now_v7().simple().to_string()[20..28]),
    )
    .await;
    let r1 = mk_unit(&pool, ag, "R1", UnitKind::Region, None, "2026-01-01").await;
    let r2 = mk_unit(&pool, ag, "R2", UnitKind::Region, None, "2026-01-01").await;
    let br = mk_unit(&pool, ag, "BR", UnitKind::Branch, Some(r1), "2026-01-01").await;

    // reparent branch r1 -> r2 effective April
    let store = store(&pool);
    let mut tx = pool.begin().await.unwrap();
    change_parent(
        &store,
        &mut tx,
        ChangeParent {
            unit_id: br,
            new_parent_id: Some(r2),
            effective_from: d("2026-04-01"),
            expected_version: 1,
            principal: principal(),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // Feb: still under r1; May: under r2
    let feb = store.ancestors_as_of(br, d("2026-02-15")).await.unwrap();
    assert!(
        feb.contains(&r1) && !feb.contains(&r2),
        "Feb hierarchy wrong: {feb:?}"
    );
    let may = store.ancestors_as_of(br, d("2026-05-15")).await.unwrap();
    assert!(
        may.contains(&r2) && !may.contains(&r1),
        "May hierarchy wrong: {may:?}"
    );
}

// ---------- assignments & transfers ----------

#[tokio::test]
async fn assignment_overlap_rejected_and_transfer_preserves_history() {
    let pool = pool().await;
    let ag = mk_agency(
        &pool,
        &format!("AG{}", &Uuid::now_v7().simple().to_string()[20..28]),
    )
    .await;
    let ba = mk_unit(&pool, ag, "BA", UnitKind::Branch, None, "2026-01-01").await;
    let bb = mk_unit(&pool, ag, "BB", UnitKind::Branch, None, "2026-01-01").await;
    let agent = EntityId::new();
    let store = store(&pool);
    let _ = &store;

    // primary assignment to BA from Jan
    let mut tx = pool.begin().await.unwrap();
    let a1 = assign_agent(
        &store,
        &mut tx,
        AssignAgent {
            agent_id: agent,
            unit_id: ba,
            assignment_type: AssignmentType::Primary,
            effective_from: d("2026-01-01"),
            principal: principal(),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // overlapping second primary -> reject
    let mut tx = pool.begin().await.unwrap();
    let err = assign_agent(
        &store,
        &mut tx,
        AssignAgent {
            agent_id: agent,
            unit_id: bb,
            assignment_type: AssignmentType::Primary,
            effective_from: d("2026-02-01"),
            principal: principal(),
        },
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, AppError::Conflict(_)),
        "overlap must fail: {err:?}"
    );
    drop(tx);

    // transfer to BB effective April: ends BA at Apr, creates BB
    let mut tx = pool.begin().await.unwrap();
    let (_new_a, transfer) = transfer_agent(
        &store,
        &mut tx,
        TransferAgent {
            agent_id: agent,
            to_unit_id: bb,
            effective_from: d("2026-04-01"),
            reason: Some("promotion".into()),
            principal: principal(),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(transfer.agent_id, agent);

    // history: March -> BA, May -> BB
    let mar = store
        .assignments_for_agent_as_of(agent, d("2026-03-15"))
        .await
        .unwrap();
    assert!(mar.iter().any(|a| a.unit_id == ba), "March should show BA");
    let may = store
        .assignments_for_agent_as_of(agent, d("2026-05-15"))
        .await
        .unwrap();
    assert!(may.iter().any(|a| a.unit_id == bb), "May should show BB");
    assert!(
        may.iter().all(|a| a.unit_id != ba),
        "old assignment must be ended"
    );
    let _ = a1;
}

// ---------- concurrency ----------

#[tokio::test]
async fn concurrent_reparent_one_wins_or_both_valid() {
    let pool = pool().await;
    let ag = mk_agency(
        &pool,
        &format!("AG{}", &Uuid::now_v7().simple().to_string()[20..28]),
    )
    .await;
    let r = mk_unit(&pool, ag, "R", UnitKind::Region, None, "2026-01-01").await;
    let b1 = mk_unit(&pool, ag, "B1", UnitKind::Branch, Some(r), "2026-01-01").await;
    let b2 = mk_unit(&pool, ag, "B2", UnitKind::Branch, Some(r), "2026-01-01").await;

    // Two concurrent reparents of b1 under b2's subtree is impossible here;
    // instead race two reparents of DIFFERENT units under each other is a
    // cycle scenario: b1->b2 and b2->b1 concurrently. At least one must fail.
    let store = store(&pool);
    let p = principal();
    let (res1, res2) = tokio::join!(
        async {
            let mut tx = pool.begin().await.unwrap();
            let r = change_parent(
                &store,
                &mut tx,
                ChangeParent {
                    unit_id: b1,
                    new_parent_id: Some(b2),
                    effective_from: d("2026-03-01"),
                    expected_version: 1,
                    principal: p.clone(),
                },
            )
            .await;
            if r.is_ok() {
                tx.commit().await.unwrap();
            }
            r.map(|_| ())
        },
        async {
            let mut tx = pool.begin().await.unwrap();
            let r = change_parent(
                &store,
                &mut tx,
                ChangeParent {
                    unit_id: b2,
                    new_parent_id: Some(b1),
                    effective_from: d("2026-03-01"),
                    expected_version: 1,
                    principal: p.clone(),
                },
            )
            .await;
            if r.is_ok() {
                tx.commit().await.unwrap();
            }
            r.map(|_| ())
        },
    );
    // Not both can succeed (would create a 2-cycle) — at least one errors.
    assert!(
        res1.is_err() || res2.is_err(),
        "mutual reparent must not both succeed"
    );
}

// ---------- idempotency ----------

#[tokio::test]
async fn idempotency_replay_and_conflict() {
    let pool = pool().await;
    let idem = PgIdempotencyStore::new();
    let mut tx = pool.begin().await.unwrap();
    let fp = "fingerprint-1";
    let key = format!("key-{}", Uuid::now_v7());
    let r1 = idem
        .reserve(&mut tx, "subj", "op", &key, fp, Duration::from_secs(3600))
        .await
        .unwrap();
    let IdempotencyReserve::Acquired { record_id } = r1 else {
        panic!("first reserve must acquire")
    };
    idem.complete(&mut tx, record_id, 201, serde_json::json!({"ok": true}))
        .await
        .unwrap();
    tx.commit().await.unwrap();

    // replay: same key+fingerprint returns stored response
    let mut tx = pool.begin().await.unwrap();
    let r2 = idem
        .reserve(&mut tx, "subj", "op", &key, fp, Duration::from_secs(3600))
        .await
        .unwrap();
    assert!(matches!(r2, IdempotencyReserve::Replay { status: 201, .. }));
    drop(tx);

    // conflict: same key, different fingerprint
    let mut tx = pool.begin().await.unwrap();
    let r3 = idem
        .reserve(
            &mut tx,
            "subj",
            "op",
            &key,
            "fingerprint-2",
            Duration::from_secs(3600),
        )
        .await
        .unwrap();
    assert!(matches!(r3, IdempotencyReserve::Conflict));
    drop(tx);
}

// ---------- leadership ----------

#[tokio::test]
async fn leadership_assign_end_and_duplicate_role_rejected() {
    let pool = pool().await;
    let ag = mk_agency(
        &pool,
        &format!("AG{}", &Uuid::now_v7().simple().to_string()[20..28]),
    )
    .await;
    let unit = mk_unit(&pool, ag, "U1", UnitKind::Branch, None, "2026-01-01").await;
    let agent = EntityId::new();
    let store = store(&pool);

    let mut tx = pool.begin().await.unwrap();
    let dto = assign_leader(
        &store,
        &mut tx,
        AssignLeader {
            unit_id: unit,
            agent_id: agent,
            role: iaop_agency_service_domain::leadership::LeadershipRole::UnitHead,
            effective_from: d("2026-01-01"),
            principal: principal(),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();

    // duplicate open role rejected
    let mut tx = pool.begin().await.unwrap();
    let err = assign_leader(
        &store,
        &mut tx,
        AssignLeader {
            unit_id: unit,
            agent_id: EntityId::new(),
            role: iaop_agency_service_domain::leadership::LeadershipRole::UnitHead,
            effective_from: d("2026-02-01"),
            principal: principal(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(err, AppError::Conflict(_)));
    drop(tx);

    // end it
    let mut tx = pool.begin().await.unwrap();
    let ended = end_leader(
        &store,
        &mut tx,
        EndLeader {
            appointment_id: dto.id,
            effective_to: d("2026-06-01"),
            expected_version: dto.version,
            principal: principal(),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_eq!(ended.status, "ENDED");

    // after end, same role can be reassigned
    let mut tx = pool.begin().await.unwrap();
    let dto2 = assign_leader(
        &store,
        &mut tx,
        AssignLeader {
            unit_id: unit,
            agent_id: EntityId::new(),
            role: iaop_agency_service_domain::leadership::LeadershipRole::UnitHead,
            effective_from: d("2026-06-01"),
            principal: principal(),
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    assert_ne!(dto2.id, dto.id);
}

#[tokio::test]
async fn outbox_lease_claiming_fenced_completion_and_poison_handling() {
    use iaop_agency_service_infrastructure::outbox::{claim_batch, mark_published, mark_retry};

    let pool = pool().await;
    let event_id = Uuid::now_v7();

    // Insert an unpublished test event
    sqlx::query(
        r#"
        INSERT INTO outbox_events (event_id, aggregate_id, event_type, event_version, payload, occurred_at)
        VALUES ($1, $1, 'test.event.v1', 1, '{"test": true}', now())
        "#,
    )
    .bind(event_id)
    .execute(&pool)
    .await
    .unwrap();

    let instance_a = "worker-instance-a";
    let instance_b = "worker-instance-b";

    // 1. Worker A claims the batch
    let mut tx_a1 = pool.begin().await.unwrap();
    let batch = claim_batch(&mut tx_a1, instance_a, 30, 1000).await.unwrap();
    tx_a1.commit().await.unwrap();
    assert!(batch.iter().any(|r| r.event_id == event_id));

    // 2. Fencing: Worker B attempting to complete Worker A's lease must fail
    let mut tx_b = pool.begin().await.unwrap();
    let published_by_b = mark_published(&mut tx_b, event_id, instance_b)
        .await
        .unwrap();
    tx_b.commit().await.unwrap();
    assert!(
        !published_by_b,
        "Worker B must not complete Worker A's lease"
    );

    // 3. Worker A completes successfully
    let mut tx_a2 = pool.begin().await.unwrap();
    let published_by_a = mark_published(&mut tx_a2, event_id, instance_a)
        .await
        .unwrap();
    tx_a2.commit().await.unwrap();
    assert!(published_by_a, "Worker A holding lease must succeed");

    // Verify event is now published
    let is_published: bool = sqlx::query_scalar(
        "SELECT published_at IS NOT NULL FROM outbox_events WHERE event_id = $1",
    )
    .bind(event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(is_published);

    // 4. Poison handling: insert event with 4 previous attempts
    let poison_id = Uuid::now_v7();
    sqlx::query(
        r#"
        INSERT INTO outbox_events (event_id, aggregate_id, event_type, event_version, payload, occurred_at, publish_attempts)
        VALUES ($1, $1, 'poison.event.v1', 1, '{"poison": true}', now(), 4)
        "#,
    )
    .bind(poison_id)
    .execute(&pool)
    .await
    .unwrap();

    // Claim poison event
    let mut tx_p1 = pool.begin().await.unwrap();
    let poison_batch = claim_batch(&mut tx_p1, instance_a, 30, 1000).await.unwrap();
    tx_p1.commit().await.unwrap();
    assert!(poison_batch.iter().any(|r| r.event_id == poison_id));

    // Mark retry (5th attempt hits max_attempts = 5 -> terminal)
    let mut tx_p2 = pool.begin().await.unwrap();
    let retried = mark_retry(
        &mut tx_p2,
        poison_id,
        instance_a,
        5,
        5,
        "permanent Kafka serialization failure",
    )
    .await
    .unwrap();
    tx_p2.commit().await.unwrap();
    assert!(retried);

    // Verify is_terminal is true and next_attempt_at is NULL
    let (is_term, next_attempt): (bool, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT is_terminal, next_attempt_at FROM outbox_events WHERE event_id = $1",
    )
    .bind(poison_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(is_term, "Poison event must be marked terminal");
    assert!(
        next_attempt.is_none(),
        "Terminal poison event must not have next_attempt_at set"
    );

    // Terminal event must not be claimed by subsequent claim_batch
    let mut tx_p3 = pool.begin().await.unwrap();
    let fresh_batch = claim_batch(&mut tx_p3, instance_b, 30, 1000).await.unwrap();
    tx_p3.commit().await.unwrap();
    assert!(
        !fresh_batch.iter().any(|r| r.event_id == poison_id),
        "Terminal events must never be re-leased"
    );
}

#[tokio::test]
async fn resource_authz_negative_tests() {
    let pool = pool().await;
    let store = store(&pool);

    // 1. Setup Agency 1 + Unit 1, and Agency 2 + Unit 2
    let agency_1 = mk_agency(&pool, "AG-AUTHZ-1").await;
    let unit_1 = mk_unit(&pool, agency_1, "U1", UnitKind::Region, None, "2026-01-01").await;

    let agency_2 = mk_agency(&pool, "AG-AUTHZ-2").await;
    let unit_2 = mk_unit(&pool, agency_2, "U2", UnitKind::Region, None, "2026-01-01").await;

    let agent_a = EntityId::new();
    let agent_b = EntityId::new();

    // Assign Agent A -> Unit 1 (Active)
    let mut tx = pool.begin().await.unwrap();
    let mut assign_a = iaop_agency_service_domain::assignment::Assignment::create(
        agent_a,
        unit_1,
        iaop_agency_service_domain::assignment::AssignmentType::Primary,
        d("2026-01-01"),
    )
    .unwrap();
    assign_a.activate(d("2026-01-01")).unwrap();
    store.insert_assignment(&mut tx, &assign_a).await.unwrap();
    tx.commit().await.unwrap();

    // Assign Agent B -> Unit 2 (Active)
    let mut tx2 = pool.begin().await.unwrap();
    let mut assign_b = iaop_agency_service_domain::assignment::Assignment::create(
        agent_b,
        unit_2,
        iaop_agency_service_domain::assignment::AssignmentType::Primary,
        d("2026-01-01"),
    )
    .unwrap();
    assign_b.activate(d("2026-01-01")).unwrap();
    store.insert_assignment(&mut tx2, &assign_b).await.unwrap();
    tx2.commit().await.unwrap();

    // Principal for Agent A (non-staff, has agent:read & agency:read)
    let principal_a = Principal {
        subject: agent_a.to_string(),
        issuer: "test".into(),
        scopes: vec![],
        roles: vec![
            "agency:read".into(),
            "agency:assignment:read".into(),
            "agency:transfer:request".into(),
        ],
        agent_scope: Some(agent_a.to_string()),
    };

    // Principal for Agent B
    let principal_b = Principal {
        subject: agent_b.to_string(),
        issuer: "test".into(),
        scopes: vec![],
        roles: vec![
            "agency:read".into(),
            "agency:assignment:read".into(),
            "agency:transfer:request".into(),
        ],
        agent_scope: Some(agent_b.to_string()),
    };

    // A. Agent A can read Agency 1 and Unit 1 (own scope)
    let res_agency_1 = iaop_agency_service_application::queries::get_agency(&store, agency_1, &principal_a).await;
    assert!(res_agency_1.is_ok(), "Agent A must access own assigned agency");

    let res_unit_1 = iaop_agency_service_application::queries::get_unit(&store, unit_1, &principal_a).await;
    assert!(res_unit_1.is_ok(), "Agent A must access own assigned unit");

    // B. NEGATIVE: Agent A cannot read Agency 2 or Unit 2 (unrelated scope) -> 403 Forbidden
    let res_agency_2 = iaop_agency_service_application::queries::get_agency(&store, agency_2, &principal_a).await;
    assert!(
        matches!(res_agency_2, Err(iaop_agency_service_application::AppError::Forbidden(_))),
        "Agent A must NOT access unrelated Agency 2"
    );

    let res_unit_2 = iaop_agency_service_application::queries::get_unit(&store, unit_2, &principal_a).await;
    assert!(
        matches!(res_unit_2, Err(iaop_agency_service_application::AppError::Forbidden(_))),
        "Agent A must NOT access unrelated Unit 2"
    );

    // C. NEGATIVE: Agent A cannot read Agent B's assignments -> 403 Forbidden
    let res_agent_b_assign = iaop_agency_service_application::queries::agent_assignments_as_of(&store, agent_b, None, &principal_a).await;
    assert!(
        matches!(res_agent_b_assign, Err(iaop_agency_service_application::AppError::Forbidden(_))),
        "Agent A must NOT access Agent B assignments"
    );

    // D. Self-service: Agent A CAN read own assignments
    let res_agent_a_assign = iaop_agency_service_application::queries::agent_assignments_as_of(&store, agent_a, None, &principal_a).await;
    assert!(res_agent_a_assign.is_ok(), "Agent A can access own assignments");
    assert_eq!(res_agent_a_assign.unwrap().len(), 1);

    // E. NEGATIVE: Agent A cannot read Unit 2's assignments
    let res_unit_2_assign = iaop_agency_service_application::queries::unit_assignments_as_of(&store, unit_2, None, &principal_a).await;
    assert!(
        matches!(res_unit_2_assign, Err(iaop_agency_service_application::AppError::Forbidden(_))),
        "Agent A must NOT access Unit 2 assignments"
    );

    // F. NEGATIVE: Agent A cannot access transfer history of Agent B
    let res_transfer_b = iaop_agency_service_application::queries::transfer_history(&store, Some(agent_b), 10, &principal_a).await;
    assert!(
        matches!(res_transfer_b, Err(iaop_agency_service_application::AppError::Forbidden(_))),
        "Agent A must NOT access transfer history of Agent B"
    );

    // F2. Positive control: Agent B CAN read own Agency 2 / Unit 2
    assert!(
        iaop_agency_service_application::queries::get_agency(&store, agency_2, &principal_b)
            .await
            .is_ok(),
        "Agent B must access own assigned agency"
    );

    // G. NEGATIVE: Pending or Ended assignment does not grant access
    let agent_pending = EntityId::new();
    let mut tx_pend = pool.begin().await.unwrap();
    let assign_pending = iaop_agency_service_domain::assignment::Assignment {
        id: EntityId::new(),
        agent_id: agent_pending,
        unit_id: unit_1,
        assignment_type: iaop_agency_service_domain::assignment::AssignmentType::Primary,
        status: iaop_agency_service_domain::assignment::AssignmentStatus::Pending,
        effective_from: d("2026-01-01"),
        effective_to: None,
        version: 0,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    store.insert_assignment(&mut tx_pend, &assign_pending).await.unwrap();
    tx_pend.commit().await.unwrap();

    let principal_pending = Principal {
        subject: agent_pending.to_string(),
        issuer: "test".into(),
        scopes: vec![],
        roles: vec!["agency:read".into()],
        agent_scope: Some(agent_pending.to_string()),
    };

    let res_pending_read = iaop_agency_service_application::queries::get_agency(&store, agency_1, &principal_pending).await;
    assert!(
        matches!(res_pending_read, Err(iaop_agency_service_application::AppError::Forbidden(_))),
        "Pending assignment must NOT grant organizational access"
    );
}
