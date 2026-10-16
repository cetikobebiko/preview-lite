/** Backend failure code for a multi-file export whose planned names already exist. */
export const OUTPUT_CONFLICT_CODE = "SESSION_JOB_OUTPUT_CONFLICT";

const MAX_LISTED_NAMES = 8;

/** Conflicting file names of an output-conflict job error, otherwise `null`. */
export function outputConflictNames(error) {
  if (error?.code !== OUTPUT_CONFLICT_CODE) return null;
  const names = error.jobOutcome?.error?.conflictingNames;
  return Array.isArray(names) ? names : [];
}

export function outputConflictMessage(names) {
  const listed = names.slice(0, MAX_LISTED_NAMES).map((name) => `• ${name}`);
  if (names.length > MAX_LISTED_NAMES) {
    listed.push(`… ve ${names.length - MAX_LISTED_NAMES} dosya daha`);
  }
  return [
    `Seçilen klasörde aynı adlı ${names.length} dosya zaten var:`,
    ...listed,
    "",
    "Bu dosyaların üzerine yazılsın mı?",
  ].join("\n");
}

/**
 * Runs `publish(false)`; when the backend reports existing files, asks
 * `confirmOverwrite(names)` and retries once with `publish(true)`.
 * Returns `{ status: "completed", value }` or `{ status: "declined", names }`.
 * Any other error is rethrown unchanged.
 */
export async function publishWithOverwriteConfirmation(publish, confirmOverwrite) {
  try {
    return { status: "completed", value: await publish(false) };
  } catch (error) {
    const names = outputConflictNames(error);
    if (names === null) throw error;
    if (!(await confirmOverwrite(names))) return { status: "declined", names };
    return { status: "completed", value: await publish(true) };
  }
}
