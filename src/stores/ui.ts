import { reactive } from "vue";

/** Sidebar navigation ids (view switching, no router). */
export type ViewId = "share" | "audience" | "network" | "quality" | "log";

/** Transient UI state (not persisted). */
export const ui = reactive({
  view: "share" as ViewId,
  aboutOpen: false,
});

export function switchView(view: ViewId): void {
  ui.view = view;
}
