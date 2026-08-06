/** Routes registered commands without owning feature or UI state. */
export function createCommandActionRouter({
  getCommand,
  actions,
  guard,
  featureContext,
  reportUnavailable,
  onUnknown,
}) {
  async function dispatch(commandId) {
    const command = getCommand(commandId);
    const action = command ? actions[command.id] : null;
    if (!command || typeof action !== "function") {
      onUnknown(commandId);
      return { status: "unknown" };
    }
    try {
      if (command.featureId) await guard(command.featureId, featureContext, action);
      else await action();
      return { status: "handled" };
    } catch (error) {
      if (!reportUnavailable(error)) throw error;
      return { status: "unavailable" };
    }
  }

  return Object.freeze({
    dispatch,
    handleEvent: (event) => dispatch(event?.detail),
    commandIds: Object.freeze(Object.keys(actions)),
  });
}
