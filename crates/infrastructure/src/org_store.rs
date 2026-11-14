//! PostgreSQL implementation of [`OrgStore`].
//! Mutations use optimistic `version` checks; relationship writes are
//! temporal (close + insert) within the caller's transaction boundary
//! where the API layer provides one — store methods run on the pool.

use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use iaop_agency_service_application::error::AppError;
use iaop_agency_service_application::ports::{AuditRecord, OrgStore, OutboxEvent, Tx, UnitView};
use iaop_agency_service_domain::agency::{Agency, AgencyStatus};
use iaop_agency_service_domain::assignment::{
    Assignment, AssignmentStatus, AssignmentType, Transfer,
};
use iaop_agency_service_domain::entity_id::EntityId;
use iaop_agency_service_domain::leadership::{
    LeadershipAppointment, LeadershipRole, LeadershipStatus,
};
use iaop_agency_service_domain::org_unit::{OrgRelationship, OrgUnit, UnitKind, UnitStatus};

pub struct PgOrgStore;

impl PgOrgStore {
    pub fn new() -> Self {
        Self
    }
}

impl Default for PgOrgStore {
    fn default() -> Self {
        Self::new()
    }
}

fn eid(id: EntityId) -> Uuid {
    id.as_uuid()
}
fn ent(id: Uuid) -> EntityId {
    EntityId::from_uuid(id)
}

fn db_err(e: sqlx::Error) -> AppError {
    AppError::infra(e)
}

fn conflict(msg: &str) -> AppError {
    AppError::Conflict(msg.to_string())
}

// ---------- row mapping ----------

fn agency_row(r: &sqlx::postgres::PgRow) -> Agency {
    Agency {
        id: ent(r.get("id")),
        code: r.get("code"),
        name: r.get("name"),
        status: match r.get::<String, _>("status").as_str() {
            "SUSPENDED" => AgencyStatus::Suspended,
            "TERMINATED" => AgencyStatus::Terminated,
            _ => AgencyStatus::Active,
        },
        version: r.get("version"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn unit_row(r: &sqlx::postgres::PgRow) -> OrgUnit {
    OrgUnit {
        id: ent(r.get("id")),
        agency_id: ent(r.get("agency_id")),
        code: r.get("code"),
        name: r.get("name"),
        kind: match r.get::<String, _>("kind").as_str() {
            "REGION" => UnitKind::Region,
            "DEPARTMENT" => UnitKind::Department,
            "TEAM" => UnitKind::Team,
            _ => UnitKind::Branch,
        },
        status: match r.get::<String, _>("status").as_str() {
            "INACTIVE" => UnitStatus::Inactive,
            _ => UnitStatus::Active,
        },
        version: r.get("version"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn assignment_row(r: &sqlx::postgres::PgRow) -> Assignment {
    Assignment {
        id: ent(r.get("id")),
        agent_id: ent(r.get("agent_id")),
        unit_id: ent(r.get("unit_id")),
        assignment_type: match r.get::<String, _>("assignment_type").as_str() {
            "SECONDARY" => AssignmentType::Secondary,
            _ => AssignmentType::Primary,
        },
        status: match r.get::<String, _>("status").as_str() {
            "ACTIVE" => AssignmentStatus::Active,
            "ENDED" => AssignmentStatus::Ended,
            "CANCELLED" => AssignmentStatus::Cancelled,
            _ => AssignmentStatus::Pending,
        },
        effective_from: r.get("effective_from"),
        effective_to: r.get("effective_to"),
        version: r.get("version"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

fn transfer_row(r: &sqlx::postgres::PgRow) -> Transfer {
    Transfer {
        id: ent(r.get("id")),
        agent_id: ent(r.get("agent_id")),
        from_unit_id: r.get::<Option<Uuid>, _>("from_unit_id").map(ent),
        to_unit_id: ent(r.get("to_unit_id")),
        effective_from: r.get("effective_from"),
        reason: r.get("reason"),
        status: match r.get::<String, _>("status").as_str() {
            "ACTIVE" => AssignmentStatus::Active,
            "ENDED" => AssignmentStatus::Ended,
            "CANCELLED" => AssignmentStatus::Cancelled,
            _ => AssignmentStatus::Pending,
        },
        version: r.get("version"),
        created_at: r.get("created_at"),
    }
}

fn leadership_row(r: &sqlx::postgres::PgRow) -> LeadershipAppointment {
    LeadershipAppointment {
        id: ent(r.get("id")),
        unit_id: ent(r.get("unit_id")),
        agent_id: ent(r.get("agent_id")),
        role: LeadershipRole::parse(&r.get::<String, _>("role"))
            .unwrap_or(LeadershipRole::UnitHead),
        status: match r.get::<String, _>("status").as_str() {
            "ACTIVE" => LeadershipStatus::Active,
            "ENDED" => LeadershipStatus::Ended,
            "CANCELLED" => LeadershipStatus::Cancelled,
            _ => LeadershipStatus::Pending,
        },
        effective_from: r.get("effective_from"),
        effective_to: r.get("effective_to"),
        version: r.get("version"),
        created_at: r.get("created_at"),
        updated_at: r.get("updated_at"),
    }
}

use sqlx::Row;

// OrgStore methods take &self with no pool — pool is passed via a
// wrapper. To keep the trait object-safe-free and simple, PgOrgStore
// methods are implemented on a struct holding the pool.
pub struct PgOrgStoreWithPool {
    pub pool: PgPool,
}

impl OrgStore for PgOrgStoreWithPool {
    // agencies
    async fn insert_agency(&self, tx: &mut Tx<'_>, a: &Agency) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO agencies (id, code, name, status, version, created_at, updated_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(eid(a.id))
        .bind(&a.code)
        .bind(&a.name)
        .bind(format!("{:?}", a.status).to_uppercase())
        .bind(a.version)
        .bind(a.created_at)
        .bind(a.updated_at)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_agency(&self, id: EntityId) -> Result<Option<Agency>, AppError> {
        let row = sqlx::query("SELECT * FROM agencies WHERE id=$1")
            .bind(eid(id))
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(agency_row))
    }

    async fn get_agency_by_code(&self, code: &str) -> Result<Option<Agency>, AppError> {
        let row = sqlx::query("SELECT * FROM agencies WHERE code=$1")
            .bind(code)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(agency_row))
    }

    async fn list_agencies(
        &self,
        limit: u32,
        cursor: Option<Uuid>,
    ) -> Result<Vec<Agency>, AppError> {
        let rows = sqlx::query(
            "SELECT * FROM agencies WHERE ($1::uuid IS NULL OR id > $1)
             ORDER BY id LIMIT $2",
        )
        .bind(cursor)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.iter().map(agency_row).collect())
    }

    async fn update_agency(
        &self,
        tx: &mut Tx<'_>,
        a: &Agency,
        expected_version: i64,
    ) -> Result<(), AppError> {
        let n = sqlx::query(
            "UPDATE agencies SET name=$2, status=$3, version=$4, updated_at=$5
             WHERE id=$1 AND version=$6",
        )
        .bind(eid(a.id))
        .bind(&a.name)
        .bind(format!("{:?}", a.status).to_uppercase())
        .bind(a.version)
        .bind(a.updated_at)
        .bind(expected_version)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?
        .rows_affected();
        if n == 0 {
            return Err(conflict("agency version conflict"));
        }
        Ok(())
    }

    // org units
    async fn insert_unit(&self, tx: &mut Tx<'_>, u: &OrgUnit) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO organizational_units (id, agency_id, code, name, kind, status, version, created_at, updated_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(eid(u.id))
        .bind(eid(u.agency_id))
        .bind(&u.code)
        .bind(&u.name)
        .bind(format!("{:?}", u.kind).to_uppercase())
        .bind(format!("{:?}", u.status).to_uppercase())
        .bind(u.version)
        .bind(u.created_at)
        .bind(u.updated_at)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_unit(&self, id: EntityId) -> Result<Option<OrgUnit>, AppError> {
        let row = sqlx::query("SELECT * FROM organizational_units WHERE id=$1")
            .bind(eid(id))
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(unit_row))
    }

    async fn list_units(
        &self,
        agency_id: Option<EntityId>,
        limit: u32,
        cursor: Option<Uuid>,
    ) -> Result<Vec<UnitView>, AppError> {
        let rows = sqlx::query(
            "SELECT u.*, p.parent_id FROM organizational_units u
             LEFT JOIN LATERAL (
               SELECT parent_id FROM organization_relationships
               WHERE child_id = u.id AND effective_to IS NULL
               ORDER BY effective_from DESC LIMIT 1
             ) p ON true
             WHERE ($1::uuid IS NULL OR u.agency_id=$1)
               AND ($2::uuid IS NULL OR u.id > $2)
             ORDER BY u.id LIMIT $3",
        )
        .bind(agency_id.map(eid))
        .bind(cursor)
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows
            .iter()
            .map(|r| UnitView {
                unit: unit_row(r),
                parent_id: r.get::<Option<Uuid>, _>("parent_id").map(ent),
            })
            .collect())
    }

    async fn update_unit(
        &self,
        tx: &mut Tx<'_>,
        u: &OrgUnit,
        expected_version: i64,
    ) -> Result<(), AppError> {
        let n = sqlx::query(
            "UPDATE organizational_units SET name=$2, kind=$3, status=$4, version=$5, updated_at=$6
             WHERE id=$1 AND version=$7",
        )
        .bind(eid(u.id))
        .bind(&u.name)
        .bind(format!("{:?}", u.kind).to_uppercase())
        .bind(format!("{:?}", u.status).to_uppercase())
        .bind(u.version)
        .bind(u.updated_at)
        .bind(expected_version)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?
        .rows_affected();
        if n == 0 {
            return Err(conflict("unit version conflict"));
        }
        Ok(())
    }

    // relationships
    async fn insert_relationship(
        &self,
        tx: &mut Tx<'_>,
        r: &OrgRelationship,
    ) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO organization_relationships (id, parent_id, child_id, effective_from, effective_to, created_at)
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(eid(r.id))
        .bind(eid(r.parent_id))
        .bind(eid(r.child_id))
        .bind(r.effective_from)
        .bind(r.effective_to)
        .bind(r.created_at)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn close_relationship(
        &self,
        tx: &mut Tx<'_>,
        child_id: EntityId,
        effective_to: NaiveDate,
    ) -> Result<(), AppError> {
        sqlx::query(
            "UPDATE organization_relationships SET effective_to=$2
             WHERE child_id=$1 AND effective_to IS NULL",
        )
        .bind(eid(child_id))
        .bind(effective_to)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn current_parent(&self, unit_id: EntityId) -> Result<Option<EntityId>, AppError> {
        let row = sqlx::query(
            "SELECT parent_id FROM organization_relationships
             WHERE child_id=$1 AND effective_to IS NULL
             ORDER BY effective_from DESC LIMIT 1",
        )
        .bind(eid(unit_id))
        .fetch_optional(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(row.and_then(|r| r.get::<Option<Uuid>, _>(0).map(ent)))
    }

    async fn ancestors(&self, unit_id: EntityId) -> Result<Vec<EntityId>, AppError> {
        self.ancestors_as_of(unit_id, chrono::Utc::now().date_naive())
            .await
    }

    async fn descendants(&self, unit_id: EntityId) -> Result<Vec<EntityId>, AppError> {
        let rows = sqlx::query(
            "WITH RECURSIVE tree AS (
               SELECT child_id FROM organization_relationships
               WHERE parent_id=$1 AND effective_to IS NULL
               UNION ALL
               SELECT r.child_id FROM organization_relationships r
               JOIN tree t ON r.parent_id = t.child_id
               WHERE r.effective_to IS NULL
             ) SELECT child_id FROM tree",
        )
        .bind(eid(unit_id))
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.iter().map(|r| ent(r.get("child_id"))).collect())
    }

    async fn children_as_of(
        &self,
        unit_id: EntityId,
        as_of: NaiveDate,
    ) -> Result<Vec<EntityId>, AppError> {
        let rows = sqlx::query(
            "SELECT child_id FROM organization_relationships
             WHERE parent_id=$1 AND effective_from <= $2
               AND (effective_to IS NULL OR effective_to > $2)",
        )
        .bind(eid(unit_id))
        .bind(as_of)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.iter().map(|r| ent(r.get("child_id"))).collect())
    }

    async fn ancestors_as_of(
        &self,
        unit_id: EntityId,
        as_of: NaiveDate,
    ) -> Result<Vec<EntityId>, AppError> {
        // Walk up using relationships covering as_of.
        let mut out = Vec::new();
        let mut cur = Some(unit_id);
        let mut depth = 0;
        while let Some(id) = cur {
            if depth > 64 {
                return Err(AppError::infra_msg("hierarchy depth exceeded"));
            }
            let row = sqlx::query(
                "SELECT parent_id FROM organization_relationships
                 WHERE child_id=$1 AND effective_from <= $2
                   AND (effective_to IS NULL OR effective_to > $2)
                 ORDER BY effective_from DESC LIMIT 1",
            )
            .bind(eid(id))
            .bind(as_of)
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
            match row.and_then(|r| r.get::<Option<Uuid>, _>(0)) {
                Some(p) => {
                    out.push(ent(p));
                    cur = Some(ent(p));
                }
                None => break,
            }
            depth += 1;
        }
        Ok(out)
    }

    // assignments
    async fn insert_assignment(&self, tx: &mut Tx<'_>, a: &Assignment) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO agent_assignments (id, agent_id, unit_id, assignment_type, status,
               effective_from, effective_to, version, created_at, updated_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(eid(a.id))
        .bind(eid(a.agent_id))
        .bind(eid(a.unit_id))
        .bind(format!("{:?}", a.assignment_type).to_uppercase())
        .bind(format!("{:?}", a.status).to_uppercase())
        .bind(a.effective_from)
        .bind(a.effective_to)
        .bind(a.version)
        .bind(a.created_at)
        .bind(a.updated_at)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_assignment(&self, id: EntityId) -> Result<Option<Assignment>, AppError> {
        let row = sqlx::query("SELECT * FROM agent_assignments WHERE id=$1")
            .bind(eid(id))
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(assignment_row))
    }

    async fn update_assignment(
        &self,
        tx: &mut Tx<'_>,
        a: &Assignment,
        expected_version: i64,
    ) -> Result<(), AppError> {
        let n = sqlx::query(
            "UPDATE agent_assignments SET unit_id=$2, status=$3, effective_to=$4,
               version=$5, updated_at=$6 WHERE id=$1 AND version=$7",
        )
        .bind(eid(a.id))
        .bind(eid(a.unit_id))
        .bind(format!("{:?}", a.status).to_uppercase())
        .bind(a.effective_to)
        .bind(a.version)
        .bind(a.updated_at)
        .bind(expected_version)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?
        .rows_affected();
        if n == 0 {
            return Err(conflict("assignment version conflict"));
        }
        Ok(())
    }

    async fn assignments_for_agent_as_of(
        &self,
        agent_id: EntityId,
        as_of: NaiveDate,
    ) -> Result<Vec<Assignment>, AppError> {
        let rows = sqlx::query(
            "SELECT * FROM agent_assignments
             WHERE agent_id=$1 AND status <> 'CANCELLED'
               AND effective_from <= $2
               AND (effective_to IS NULL OR effective_to > $2)",
        )
        .bind(eid(agent_id))
        .bind(as_of)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.iter().map(assignment_row).collect())
    }

    async fn assignments_for_unit_as_of(
        &self,
        unit_id: EntityId,
        as_of: NaiveDate,
    ) -> Result<Vec<Assignment>, AppError> {
        let rows = sqlx::query(
            "SELECT * FROM agent_assignments
             WHERE unit_id=$1 AND status <> 'CANCELLED'
               AND effective_from <= $2
               AND (effective_to IS NULL OR effective_to > $2)",
        )
        .bind(eid(unit_id))
        .bind(as_of)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.iter().map(assignment_row).collect())
    }

    async fn assignments_for_agent(&self, agent_id: EntityId) -> Result<Vec<Assignment>, AppError> {
        let rows = sqlx::query(
            "SELECT * FROM agent_assignments WHERE agent_id=$1 ORDER BY effective_from",
        )
        .bind(eid(agent_id))
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.iter().map(assignment_row).collect())
    }

    async fn overlapping_primary(
        &self,
        agent_id: EntityId,
        from: NaiveDate,
        to: Option<NaiveDate>,
    ) -> Result<Vec<Assignment>, AppError> {
        // Overlap: existing.effective_from < new.to AND existing.effective_to > new.from
        let rows = sqlx::query(
            "SELECT * FROM agent_assignments
             WHERE agent_id=$1 AND assignment_type='PRIMARY' AND status <> 'CANCELLED'
               AND effective_from < COALESCE($3, 'infinity'::date)
               AND (effective_to IS NULL OR effective_to > $2)",
        )
        .bind(eid(agent_id))
        .bind(from)
        .bind(to)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.iter().map(assignment_row).collect())
    }

    // transfers
    async fn insert_transfer(&self, tx: &mut Tx<'_>, t: &Transfer) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO agency_transfers (id, agent_id, from_unit_id, to_unit_id,
               effective_from, reason, status, version, created_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(eid(t.id))
        .bind(eid(t.agent_id))
        .bind(t.from_unit_id.map(eid))
        .bind(eid(t.to_unit_id))
        .bind(t.effective_from)
        .bind(&t.reason)
        .bind(format!("{:?}", t.status).to_uppercase())
        .bind(t.version)
        .bind(t.created_at)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_transfer(&self, id: EntityId) -> Result<Option<Transfer>, AppError> {
        let row = sqlx::query("SELECT * FROM agency_transfers WHERE id=$1")
            .bind(eid(id))
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(transfer_row))
    }

    async fn list_transfers(
        &self,
        agent_id: Option<EntityId>,
        limit: u32,
    ) -> Result<Vec<Transfer>, AppError> {
        let rows = sqlx::query(
            "SELECT * FROM agency_transfers
             WHERE ($1::uuid IS NULL OR agent_id=$1)
             ORDER BY created_at DESC LIMIT $2",
        )
        .bind(agent_id.map(eid))
        .bind(limit as i64)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.iter().map(transfer_row).collect())
    }

    async fn insert_leadership(
        &self,
        tx: &mut Tx<'_>,
        a: &LeadershipAppointment,
    ) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO leadership_appointments (id, unit_id, agent_id, role, status,
               effective_from, effective_to, version, created_at, updated_at)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(eid(a.id))
        .bind(eid(a.unit_id))
        .bind(eid(a.agent_id))
        .bind(a.role.as_str())
        .bind(match a.status {
            LeadershipStatus::Pending => "PENDING",
            LeadershipStatus::Active => "ACTIVE",
            LeadershipStatus::Ended => "ENDED",
            _ => "CANCELLED",
        })
        .bind(a.effective_from)
        .bind(a.effective_to)
        .bind(a.version)
        .bind(a.created_at)
        .bind(a.updated_at)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn get_leadership(
        &self,
        id: EntityId,
    ) -> Result<Option<LeadershipAppointment>, AppError> {
        let row = sqlx::query("SELECT * FROM leadership_appointments WHERE id=$1")
            .bind(eid(id))
            .fetch_optional(&self.pool)
            .await
            .map_err(db_err)?;
        Ok(row.as_ref().map(leadership_row))
    }

    async fn update_leadership(
        &self,
        tx: &mut Tx<'_>,
        a: &LeadershipAppointment,
        expected_version: i64,
    ) -> Result<(), AppError> {
        let n = sqlx::query(
            "UPDATE leadership_appointments SET status=$2, effective_to=$3, version=$4, updated_at=$5
             WHERE id=$1 AND version=$6",
        )
        .bind(eid(a.id))
        .bind(match a.status { LeadershipStatus::Pending=>"PENDING", LeadershipStatus::Active=>"ACTIVE", LeadershipStatus::Ended=>"ENDED", _=>"CANCELLED" })
        .bind(a.effective_to).bind(a.version).bind(a.updated_at).bind(expected_version)
        .execute(&mut **tx).await.map_err(db_err)?.rows_affected();
        if n == 0 {
            return Err(conflict("leadership version conflict"));
        }
        Ok(())
    }

    async fn leaders_as_of(
        &self,
        unit_id: EntityId,
        as_of: NaiveDate,
    ) -> Result<Vec<LeadershipAppointment>, AppError> {
        let rows = sqlx::query(
            "SELECT * FROM leadership_appointments
             WHERE unit_id=$1 AND status <> 'CANCELLED'
               AND effective_from <= $2 AND (effective_to IS NULL OR effective_to > $2)",
        )
        .bind(eid(unit_id))
        .bind(as_of)
        .fetch_all(&self.pool)
        .await
        .map_err(db_err)?;
        Ok(rows.iter().map(leadership_row).collect())
    }

    async fn lock_relationships(&self, tx: &mut Tx<'_>) -> Result<(), AppError> {
        sqlx::query("LOCK TABLE organization_relationships IN SHARE ROW EXCLUSIVE MODE")
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        Ok(())
    }

    async fn insert_outbox(&self, tx: &mut Tx<'_>, e: &OutboxEvent) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO outbox_events (event_id, aggregate_id, aggregate_type, aggregate_version,
               event_type, event_version, payload, occurred_at, source, correlation_id)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(e.event_id)
        .bind(e.aggregate_id)
        .bind(&e.aggregate_type)
        .bind(e.aggregate_version)
        .bind(&e.event_type)
        .bind(e.event_version)
        .bind(serde_json::to_value(&e.payload).unwrap_or(serde_json::Value::Null))
        .bind(e.occurred_at)
        .bind(&e.source)
        .bind(e.correlation_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }

    async fn insert_audit(&self, tx: &mut Tx<'_>, a: &AuditRecord) -> Result<(), AppError> {
        sqlx::query(
            "INSERT INTO agency_audit_events (id, actor_subject, action, aggregate_type,
               aggregate_id, previous_state, new_state, effective_date, reason, correlation_id)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
        )
        .bind(a.id)
        .bind(&a.actor_subject)
        .bind(&a.action)
        .bind(&a.aggregate_type)
        .bind(a.aggregate_id)
        .bind(&a.previous_state)
        .bind(&a.new_state)
        .bind(a.effective_date)
        .bind(&a.reason)
        .bind(a.correlation_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
        Ok(())
    }
}
