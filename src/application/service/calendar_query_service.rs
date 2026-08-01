//! `CalendarQueryService` impl for [`crate::CalendarModule`].
//!
//! Hand-written (user-owned — see `metaphor.codegen.yaml`). The generated `exports/services.rs`
//! declares the `CalendarQueryService` port trait but no impl is generated for it; this file is
//! that impl. It is the seam every other module consumes calendar through.
//!
//! Split:
//! - the **standard lookups** (`get_*` / `*_exists`) delegate to the existing `GenericCrudService`
//!   (already wired on the module) and map entity → public DTO.
//! - the **custom read-port** `working_days` delegates to [`CalendarRepository::working_days`],
//!   which holds the hand-written SQL (4-layer rule: services orchestrate, repos hold SQL).
//!
//! Company scoping (ADR-0008) is NOT done here — the caller (HTTP composition root via
//! `with_request_scope`, or a job via `with_company_scope`) sets it; `find_by_id` and the repo's
//! `company_scope::fetch_all_scoped` both honour the task-local RLS fence.

use anyhow::Result;
use async_trait::async_trait;
use chrono::NaiveDate;
use uuid::Uuid;

use crate::domain::entity::{
    Calendar, CalendarBranch, CalendarDepartment, CalendarEmployee, CalendarEmployeeStatus,
    CalendarLevel, CalendarPosition, CalendarReligion,
};
// `exports::services` and `exports::types` are both private modules; their items are re-exported at
// `crate::exports::` — import through that, not the private module paths.
use crate::exports::CalendarQueryService;
use crate::exports::{
    CalendarBranchDto, CalendarBranchId, CalendarBranchSummary, CalendarDepartmentDto,
    CalendarDepartmentId, CalendarDepartmentSummary, CalendarDto, CalendarEmployeeDto,
    CalendarEmployeeId, CalendarEmployeeStatusDto, CalendarEmployeeStatusId,
    CalendarEmployeeStatusSummary, CalendarEmployeeSummary, CalendarId, CalendarLevelDto,
    CalendarLevelId, CalendarLevelSummary, CalendarPositionDto, CalendarPositionId,
    CalendarPositionSummary, CalendarReligionDto, CalendarReligionId, CalendarReligionSummary,
    CalendarSummary,
};
// The `*Id` names here are the EXPORT (public) newtypes, deliberately — the domain entity also
// defines same-named id newtypes, so we import only the entity STRUCTS above (not
// `domain::entity::*`) to avoid a name collision.
use crate::CalendarModule;

#[async_trait]
impl CalendarQueryService for CalendarModule {
    async fn get_calendar(&self, id: CalendarId) -> Result<Option<CalendarDto>> {
        let entity = self
            .calendar_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(calendar_to_dto).transpose()?)
    }

    async fn get_calendar_summary(&self, id: CalendarId) -> Result<Option<CalendarSummary>> {
        let entity = self
            .calendar_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(|e| CalendarSummary { id: CalendarId(e.id), name: e.name }))
    }

    async fn calendar_exists(&self, id: CalendarId) -> Result<bool> {
        Ok(self
            .calendar_service
            .find_by_id(&id.into_inner().to_string())
            .await?
            .is_some())
    }

    async fn get_calendar_branch(&self, id: CalendarBranchId) -> Result<Option<CalendarBranchDto>> {
        let entity = self
            .calendar_branch_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(calendar_branch_to_dto).transpose()?)
    }

    async fn get_calendar_branch_summary(
        &self,
        id: CalendarBranchId,
    ) -> Result<Option<CalendarBranchSummary>> {
        let entity = self
            .calendar_branch_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(|e| CalendarBranchSummary { id: CalendarBranchId(e.id) }))
    }

    async fn calendar_branch_exists(&self, id: CalendarBranchId) -> Result<bool> {
        Ok(self
            .calendar_branch_service
            .find_by_id(&id.into_inner().to_string())
            .await?
            .is_some())
    }

    async fn get_calendar_department(
        &self,
        id: CalendarDepartmentId,
    ) -> Result<Option<CalendarDepartmentDto>> {
        let entity = self
            .calendar_department_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(calendar_department_to_dto).transpose()?)
    }

    async fn get_calendar_department_summary(
        &self,
        id: CalendarDepartmentId,
    ) -> Result<Option<CalendarDepartmentSummary>> {
        let entity = self
            .calendar_department_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(|e| CalendarDepartmentSummary { id: CalendarDepartmentId(e.id) }))
    }

    async fn calendar_department_exists(&self, id: CalendarDepartmentId) -> Result<bool> {
        Ok(self
            .calendar_department_service
            .find_by_id(&id.into_inner().to_string())
            .await?
            .is_some())
    }

    async fn get_calendar_employee(
        &self,
        id: CalendarEmployeeId,
    ) -> Result<Option<CalendarEmployeeDto>> {
        let entity = self
            .calendar_employee_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(calendar_employee_to_dto).transpose()?)
    }

    async fn get_calendar_employee_summary(
        &self,
        id: CalendarEmployeeId,
    ) -> Result<Option<CalendarEmployeeSummary>> {
        let entity = self
            .calendar_employee_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(|e| CalendarEmployeeSummary { id: CalendarEmployeeId(e.id) }))
    }

    async fn calendar_employee_exists(&self, id: CalendarEmployeeId) -> Result<bool> {
        Ok(self
            .calendar_employee_service
            .find_by_id(&id.into_inner().to_string())
            .await?
            .is_some())
    }

    async fn get_calendar_employee_status(
        &self,
        id: CalendarEmployeeStatusId,
    ) -> Result<Option<CalendarEmployeeStatusDto>> {
        let entity = self
            .calendar_employee_status_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(calendar_employee_status_to_dto).transpose()?)
    }

    async fn get_calendar_employee_status_summary(
        &self,
        id: CalendarEmployeeStatusId,
    ) -> Result<Option<CalendarEmployeeStatusSummary>> {
        let entity = self
            .calendar_employee_status_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(|e| CalendarEmployeeStatusSummary { id: CalendarEmployeeStatusId(e.id) }))
    }

    async fn calendar_employee_status_exists(&self, id: CalendarEmployeeStatusId) -> Result<bool> {
        Ok(self
            .calendar_employee_status_service
            .find_by_id(&id.into_inner().to_string())
            .await?
            .is_some())
    }

    async fn get_calendar_level(&self, id: CalendarLevelId) -> Result<Option<CalendarLevelDto>> {
        let entity = self
            .calendar_level_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(calendar_level_to_dto).transpose()?)
    }

    async fn get_calendar_level_summary(
        &self,
        id: CalendarLevelId,
    ) -> Result<Option<CalendarLevelSummary>> {
        let entity = self
            .calendar_level_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(|e| CalendarLevelSummary { id: CalendarLevelId(e.id) }))
    }

    async fn calendar_level_exists(&self, id: CalendarLevelId) -> Result<bool> {
        Ok(self
            .calendar_level_service
            .find_by_id(&id.into_inner().to_string())
            .await?
            .is_some())
    }

    async fn get_calendar_position(
        &self,
        id: CalendarPositionId,
    ) -> Result<Option<CalendarPositionDto>> {
        let entity = self
            .calendar_position_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(calendar_position_to_dto).transpose()?)
    }

    async fn get_calendar_position_summary(
        &self,
        id: CalendarPositionId,
    ) -> Result<Option<CalendarPositionSummary>> {
        let entity = self
            .calendar_position_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(|e| CalendarPositionSummary { id: CalendarPositionId(e.id) }))
    }

    async fn calendar_position_exists(&self, id: CalendarPositionId) -> Result<bool> {
        Ok(self
            .calendar_position_service
            .find_by_id(&id.into_inner().to_string())
            .await?
            .is_some())
    }

    async fn get_calendar_religion(
        &self,
        id: CalendarReligionId,
    ) -> Result<Option<CalendarReligionDto>> {
        let entity = self
            .calendar_religion_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(calendar_religion_to_dto).transpose()?)
    }

    async fn get_calendar_religion_summary(
        &self,
        id: CalendarReligionId,
    ) -> Result<Option<CalendarReligionSummary>> {
        let entity = self
            .calendar_religion_service
            .find_by_id(&id.into_inner().to_string())
            .await?;
        Ok(entity.map(|e| CalendarReligionSummary { id: CalendarReligionId(e.id) }))
    }

    async fn calendar_religion_exists(&self, id: CalendarReligionId) -> Result<bool> {
        Ok(self
            .calendar_religion_service
            .find_by_id(&id.into_inner().to_string())
            .await?
            .is_some())
    }

    async fn working_days(
        &self,
        company_id: Uuid,
        from: NaiveDate,
        to: NaiveDate,
    ) -> Result<u32> {
        Ok(self
            .calendar_repository
            .working_days(&self.db_pool, company_id, from, to)
            .await?)
    }
}

// ─── entity → public DTO mapping ───────────────────────────────────────────────
//
// The only non-trivial conversion is `metadata`: the entity holds a typed `AuditMetadata`, the
// public DTO exposes it as an opaque `serde_json::Value` (so consumers don't depend on the internal
// audit struct's shape).

fn calendar_to_dto(e: Calendar) -> Result<CalendarDto> {
    Ok(CalendarDto {
        id: CalendarId(e.id),
        company_id: e.company_id,
        name: e.name,
        date_start: e.date_start,
        date_end: e.date_end,
        is_holiday: e.is_holiday,
        can_everyone_view: e.can_everyone_view,
        note: e.note,
        metadata: serde_json::to_value(&e.metadata)?,
    })
}

fn calendar_branch_to_dto(e: CalendarBranch) -> Result<CalendarBranchDto> {
    Ok(CalendarBranchDto {
        id: CalendarBranchId(e.id),
        calendar_id: e.calendar_id,
        company_id: e.company_id,
        branch_id: e.branch_id,
        metadata: serde_json::to_value(&e.metadata)?,
    })
}

fn calendar_department_to_dto(e: CalendarDepartment) -> Result<CalendarDepartmentDto> {
    Ok(CalendarDepartmentDto {
        id: CalendarDepartmentId(e.id),
        calendar_id: e.calendar_id,
        department_id: e.department_id,
        metadata: serde_json::to_value(&e.metadata)?,
    })
}

fn calendar_employee_to_dto(e: CalendarEmployee) -> Result<CalendarEmployeeDto> {
    Ok(CalendarEmployeeDto {
        id: CalendarEmployeeId(e.id),
        calendar_id: e.calendar_id,
        employee_id: e.employee_id,
        metadata: serde_json::to_value(&e.metadata)?,
    })
}

fn calendar_employee_status_to_dto(e: CalendarEmployeeStatus) -> Result<CalendarEmployeeStatusDto> {
    Ok(CalendarEmployeeStatusDto {
        id: CalendarEmployeeStatusId(e.id),
        calendar_id: e.calendar_id,
        employment_status: e.employment_status,
        metadata: serde_json::to_value(&e.metadata)?,
    })
}

fn calendar_level_to_dto(e: CalendarLevel) -> Result<CalendarLevelDto> {
    Ok(CalendarLevelDto {
        id: CalendarLevelId(e.id),
        calendar_id: e.calendar_id,
        level_id: e.level_id,
        metadata: serde_json::to_value(&e.metadata)?,
    })
}

fn calendar_position_to_dto(e: CalendarPosition) -> Result<CalendarPositionDto> {
    Ok(CalendarPositionDto {
        id: CalendarPositionId(e.id),
        calendar_id: e.calendar_id,
        position_id: e.position_id,
        metadata: serde_json::to_value(&e.metadata)?,
    })
}

fn calendar_religion_to_dto(e: CalendarReligion) -> Result<CalendarReligionDto> {
    Ok(CalendarReligionDto {
        id: CalendarReligionId(e.id),
        calendar_id: e.calendar_id,
        religion_id: e.religion_id,
        metadata: serde_json::to_value(&e.metadata)?,
    })
}
