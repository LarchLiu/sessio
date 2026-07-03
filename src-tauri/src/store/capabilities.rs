use anyhow::Result;

use crate::models::ProjectInfo;

use super::SessionStore;

pub trait ProjectStore {
    fn list_projects(&self) -> Result<Vec<ProjectInfo>>;
    fn add_project(
        &self,
        path: &str,
        name: Option<&str>,
        process_template_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo>;
    fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        process_template_id: Option<String>,
    ) -> Result<ProjectInfo>;
    fn archive_project(&self, project_id: &str) -> Result<()>;
}

impl<T: SessionStore + ?Sized> ProjectStore for T {
    fn list_projects(&self) -> Result<Vec<ProjectInfo>> {
        SessionStore::list_projects(self)
    }

    fn add_project(
        &self,
        path: &str,
        name: Option<&str>,
        process_template_id: String,
        enabled_stage_ids: Option<&[String]>,
    ) -> Result<ProjectInfo> {
        SessionStore::add_project(self, path, name, process_template_id, enabled_stage_ids)
    }

    fn update_project(
        &self,
        project_id: &str,
        name: Option<&str>,
        process_template_id: Option<String>,
    ) -> Result<ProjectInfo> {
        SessionStore::update_project(self, project_id, name, process_template_id)
    }

    fn archive_project(&self, project_id: &str) -> Result<()> {
        SessionStore::archive_project(self, project_id)
    }
}
