import { resolveAppGateway } from "../runtime/appRuntime";
import { getAppStorage } from "../runtime/appStorage";
import { invoke, isDesktopRuntime } from "../runtime/platformCore";
import {
  normalizeProjectDraft,
  type ProjectDraft,
  type ResearchProject,
} from "@research-assistant/core/project";

const BROWSER_STORAGE_KEY = "research-assistant.projects.v1";

export interface ProjectGateway {
  list(): Promise<readonly ResearchProject[]>;
  create(draft: ProjectDraft): Promise<ResearchProject>;
}

function readBrowserProjects(): readonly ResearchProject[] {
  const value = getAppStorage().getItem(BROWSER_STORAGE_KEY);

  if (value === null) {
    return [];
  }

  try {
    const parsed: unknown = JSON.parse(value);
    return Array.isArray(parsed)
      ? (parsed as ResearchProject[]).map((project) => ({
          ...project,
          literatureCount: project.literatureCount ?? 0,
          evidenceCount: project.evidenceCount ?? 0,
        }))
      : [];
  } catch {
    return [];
  }
}

const browserGateway: ProjectGateway = {
  async list() {
    return readBrowserProjects();
  },
  async create(input) {
    const draft = normalizeProjectDraft(input);
    const project: ResearchProject = {
      id: crypto.randomUUID(),
      title: draft.title,
      summary: draft.summary,
      createdAt: new Date().toISOString(),
      literatureCount: 0,
      evidenceCount: 0,
    };
    const projects = [project, ...readBrowserProjects()];
    getAppStorage().setItem(BROWSER_STORAGE_KEY, JSON.stringify(projects));
    return project;
  },
};

const desktopGateway: ProjectGateway = {
  async list() {
    return invoke<readonly ResearchProject[]>("list_projects");
  },
  async create(input) {
    const draft = normalizeProjectDraft(input);
    return invoke<ResearchProject>("create_project", { draft });
  },
};

export function getProjectGateway(): ProjectGateway {
  return resolveAppGateway("getProjectGateway", isDesktopRuntime() ? desktopGateway : browserGateway);
}
