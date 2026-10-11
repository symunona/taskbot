package space.tmpx.taltosbot.connection

import kotlinx.serialization.json.JsonObject

/** `Task.state` of docs/protocol.md "Tasks". */
object TaskState {
    const val QUEUED = "queued"
    const val SPAWNING = "spawning"
    const val SENT = "sent"
    const val WORKING = "working"
    const val WAITING = "waiting"
    const val DONE = "done"
    const val FAILED = "failed"
}

/** `Task` of docs/protocol.md "Tasks": one `delegate` call. Times = epoch ms. */
data class TaskInfo(
    val id: String,
    val project: String,
    val threadId: String? = null,
    val prompt: String = "",
    val state: String,
    val queuePos: Int? = null,
    val createdAt: Long,
    val startedAt: Long? = null,
    val updatedAt: Long,
    val lastActivity: String? = null,
    val answer: String? = null,
    val error: String? = null
) {
    val finished: Boolean get() = state == TaskState.DONE || state == TaskState.FAILED

    /** How long the task has run, or ran: a finished task stops at its last update. */
    fun elapsedMs(now: Long): Long = ((if (finished) updatedAt else now) - createdAt).coerceAtLeast(0L)
}

internal fun parseTask(obj: JsonObject?): TaskInfo? {
    val id = obj?.str("id")?.takeIf { it.isNotBlank() } ?: return null
    val created = obj.long("created_at") ?: 0L
    return TaskInfo(
        id = id,
        project = obj.str("project") ?: "?",
        threadId = obj.str("thread_id"),
        prompt = obj.str("prompt") ?: "",
        state = obj.str("state") ?: "?",
        queuePos = obj.long("queue_pos")?.toInt(),
        createdAt = created,
        startedAt = obj.long("started_at"),
        updatedAt = obj.long("updated_at") ?: created,
        lastActivity = obj.str("last_activity"),
        answer = obj.str("answer"),
        error = obj.str("error")
    )
}

/** [task] put into [tasks] by id, newest first. An older copy of a newer task loses (pushes can race the initial list). */
internal fun upsertTask(tasks: List<TaskInfo>, task: TaskInfo): List<TaskInfo> {
    val old = tasks.firstOrNull { it.id == task.id }
    if (old != null && (old.updatedAt > task.updatedAt || (old.finished && !task.finished))) return tasks
    return (tasks.filter { it.id != task.id } + task).sortedByDescending { it.createdAt }
}
