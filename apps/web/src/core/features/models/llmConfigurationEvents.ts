export const LLM_CONFIGURATION_CHANGED_EVENT = "research-assistant:llm-configuration-changed";

export function announceLlmConfigurationChanged(): void {
  window.dispatchEvent(new Event(LLM_CONFIGURATION_CHANGED_EVENT));
}
