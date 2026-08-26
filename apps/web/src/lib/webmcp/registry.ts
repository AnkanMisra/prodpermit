export type ToolClassification = "read-only" | "untrusted-data" | "staging" | "execution";

export type ToolDescriptor = {
  name: string;
  classification: ToolClassification;
};

type ActivityBase = {
  invocationId: string;
  toolName: string;
  timestamp: string;
};

export type ToolActivity =
  | (ActivityBase & { status: "running" })
  | (ActivityBase & { status: "succeeded" })
  | (ActivityBase & { status: "failed"; message: string })
  | (ActivityBase & { status: "canceled" });

export interface ModelContextPort {
  registerTool(
    tool: WebMCP.ModelContextTool,
    options?: WebMCP.ModelContextRegisterToolOptions
  ): Promise<void>;
}

type Registration = {
  classification: ToolClassification;
  controller: AbortController;
  promise: Promise<void>;
};

type RegistryOptions = {
  modelContext: ModelContextPort;
  onActivity?: (activity: ToolActivity) => void;
  onToolsChanged?: (tools: ToolDescriptor[]) => void;
};

export class WebMcpRegistry {
  readonly #modelContext: ModelContextPort;
  readonly #onActivity: ((activity: ToolActivity) => void) | undefined;
  readonly #onToolsChanged: ((tools: ToolDescriptor[]) => void) | undefined;
  readonly #registrations = new Map<string, Registration>();

  constructor(options: RegistryOptions) {
    this.#modelContext = options.modelContext;
    this.#onActivity = options.onActivity;
    this.#onToolsChanged = options.onToolsChanged;
  }

  async register(
    tool: WebMCP.ModelContextTool,
    classification: ToolClassification
  ): Promise<void> {
    const existing = this.#registrations.get(tool.name);
    if (existing) {
      await existing.promise;
      return;
    }

    const controller = new AbortController();
    const wrapped = this.#withActivity(tool);
    const promise = this.#modelContext.registerTool(wrapped, { signal: controller.signal });
    this.#registrations.set(tool.name, { classification, controller, promise });
    try {
      await promise;
      this.#emitToolsChanged();
    } catch (error: unknown) {
      this.#registrations.delete(tool.name);
      controller.abort();
      throw error;
    }
  }

  unregister(name: string): void {
    const registration = this.#registrations.get(name);
    if (!registration) {
      return;
    }
    registration.controller.abort();
    this.#registrations.delete(name);
    this.#emitToolsChanged();
  }

  tools(): ToolDescriptor[] {
    return [...this.#registrations.entries()]
      .map(([name, registration]) => ({ name, classification: registration.classification }))
      .toSorted((left, right) => left.name.localeCompare(right.name));
  }

  dispose(): void {
    for (const registration of this.#registrations.values()) {
      registration.controller.abort();
    }
    this.#registrations.clear();
    this.#emitToolsChanged();
  }

  #withActivity(tool: WebMCP.ModelContextTool): WebMCP.ModelContextTool {
    return {
      ...tool,
      execute: async (input, options) => {
        const invocationId = crypto.randomUUID();
        this.#emitActivity({
          invocationId,
          toolName: tool.name,
          status: "running",
          timestamp: new Date().toISOString()
        });
        try {
          const result = await tool.execute(input, options);
          this.#emitActivity({
            invocationId,
            toolName: tool.name,
            status: "succeeded",
            timestamp: new Date().toISOString()
          });
          return result;
        } catch (error: unknown) {
          if (error instanceof DOMException && error.name === "AbortError") {
            this.#emitActivity({
              invocationId,
              toolName: tool.name,
              status: "canceled",
              timestamp: new Date().toISOString()
            });
          } else {
            this.#emitActivity({
              invocationId,
              toolName: tool.name,
              status: "failed",
              message: error instanceof Error ? error.message : "Tool execution failed.",
              timestamp: new Date().toISOString()
            });
          }
          throw error;
        }
      }
    };
  }

  #emitActivity(activity: ToolActivity): void {
    this.#onActivity?.(activity);
  }

  #emitToolsChanged(): void {
    this.#onToolsChanged?.(this.tools());
  }
}
