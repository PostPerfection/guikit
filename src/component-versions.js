export async function loadComponentVersions(invoke) {
  const container = document.getElementById("component-versions");
  if (!container) return;

  try {
    const versions = await invoke("component_versions");
    container.replaceChildren(...versions.map(componentVersionRow));
  } catch (error) {
    container.textContent = `Could not read component versions: ${error}`;
  }
}

function componentVersionRow(component) {
  const row = document.createElement("div");
  row.className = "component-version";

  const name = document.createElement("span");
  name.textContent = component.name;
  const version = document.createElement("output");
  version.textContent = component.version;
  row.append(name, version);
  return row;
}
