import { getStore } from "@/lib/shell";

export type TodoStatus = "pending" | "in_progress" | "completed";

export type Todo = {
  id: string;
  title: string;
  description?: string;
  status: TodoStatus;
};

const STORE_NAME = "ai-todos";
const todosKey = (sessionId: string) => `todos:${sessionId}`;

export async function loadTodos(sessionId: string): Promise<Todo[]> {
  return (await getStore(STORE_NAME).get<Todo[]>(todosKey(sessionId))) ?? [];
}

export async function saveTodos(
  sessionId: string,
  todos: Todo[],
): Promise<void> {
  await getStore(STORE_NAME).set(todosKey(sessionId), todos);
}

export async function deleteTodos(sessionId: string): Promise<void> {
  await getStore(STORE_NAME).delete(todosKey(sessionId));
}

export function newTodoId(): string {
  return `t-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 6)}`;
}

/**
 * Validate a candidate todo list:
 *  - At most one item with status `in_progress` (anti-drift invariant).
 *  - Titles must be non-empty.
 * Returns null on valid, otherwise an error string.
 */
export function validateTodos(todos: Todo[]): string | null {
  let inProgress = 0;
  for (const t of todos) {
    if (!t.title.trim()) return "todo title cannot be empty";
    if (t.status === "in_progress") inProgress++;
  }
  if (inProgress > 1)
    return `only one todo may be in_progress at a time (got ${inProgress})`;
  return null;
}
