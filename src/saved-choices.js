const NOT_CONNECTED_SUFFIX = " (not connected)";

// a saved name the system no longer lists stays a choice, marked as gone
export function withSavedChoice(connectedChoices, savedName) {
  const savedIsMissing = savedName !== null && !connectedChoices.some((choice) => choice.name === savedName);
  if (!savedIsMissing) return connectedChoices;
  return [...connectedChoices, { name: savedName, label: `${savedName}${NOT_CONNECTED_SUFFIX}` }];
}
