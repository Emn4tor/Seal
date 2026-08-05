const UPDATE_AUTO_CHECK_KEY = "seal-update-auto-check";
const UPDATE_AUTO_INSTALL_KEY = "seal-update-auto-install";

export function getUpdateAutoCheckEnabled(): boolean {
  return localStorage.getItem(UPDATE_AUTO_CHECK_KEY) === "true";
}

export function saveUpdateAutoCheckEnabled(enabled: boolean) {
  localStorage.setItem(UPDATE_AUTO_CHECK_KEY, String(enabled));
}

export function getUpdateAutoInstallEnabled(): boolean {
  return localStorage.getItem(UPDATE_AUTO_INSTALL_KEY) === "true";
}

export function saveUpdateAutoInstallEnabled(enabled: boolean) {
  localStorage.setItem(UPDATE_AUTO_INSTALL_KEY, String(enabled));
}