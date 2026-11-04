import { definePluginEntry } from 'openclaw/plugin-sdk/plugin-entry';
import { registerTools } from './tools.js';
import { createProviderRuntime } from './provider.js';

export default definePluginEntry({
  id: 'mempalace-memory',
  name: 'MemPalace Memory',
  description: 'Marker-aware MemPalace memory search',
  kind: 'memory',
  register(api) {
    api.registerMemoryCapability({
      providerRuntime: createProviderRuntime(api.pluginConfig ?? {}),
      recallToolNames: ['memory_search', 'memory_get'],
      promptBuilder: ({ availableTools }) => [
        'Long-term memory is backed by MemPalace.',
        `Before answering about prior work, people, preferences or open threads, call ${(availableTools instanceof Set ? availableTools : new Set(availableTools ?? [])).has('memory_search') ? 'memory_search' : 'mempalace_search'}.`,
        'File continuity notes through mempalace_diary_write, which records privacy markers.',
        'Use mempalace_status for an owner-private palace overview when available.',
      ],
      flushPlanResolver: () => null,
    });
    registerTools(api);
  },
});
