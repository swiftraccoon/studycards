/**
 * Keyboard shortcuts for the study pages.
 *
 * The server marks an element with a `data-key` attribute that lists keys
 * separated by spaces, such as `data-key="a"` on the first answer or
 * `data-key="n Enter"` on the Next button. Pressing one of the keys clicks the
 * element. Every page also works without this script.
 */

/** Keys with which a browser activates the focused control. */
const ACTIVATION_KEYS: ReadonlySet<string> = new Set(["Enter", " "]);

/** Elements that a browser activates with those keys. */
const INTERACTIVE = "a, button, summary";

/** Elements in which typing must never trigger a shortcut. */
const TEXT_ENTRY = "input, select, textarea, [contenteditable]";

/** Returns the element under which `target` sits and that matches `selector`. */
function enclosing(target: EventTarget | null, selector: string): Element | null {
  return target instanceof Element ? target.closest(selector) : null;
}

/** Returns the name a `data-key` attribute uses for a pressed key. */
function keyName(event: KeyboardEvent): string {
  if (event.key === " ") {
    return "Space";
  }
  // Letters are matched without regard to Shift or Caps Lock.
  return event.key.length === 1 ? event.key.toLowerCase() : event.key;
}

/** Returns the first element bound to `key`. */
function boundElement(key: string): HTMLElement | undefined {
  for (const element of document.querySelectorAll<HTMLElement>("[data-key]")) {
    const keys = (element.dataset["key"] ?? "").split(" ");
    if (keys.includes(key)) {
      return element;
    }
  }
  return undefined;
}

document.addEventListener("keydown", (event: KeyboardEvent): void => {
  if (event.defaultPrevented || event.repeat || event.altKey || event.ctrlKey || event.metaKey) {
    return;
  }
  if (enclosing(event.target, TEXT_ENTRY) !== null) {
    return;
  }
  const element = boundElement(keyName(event));
  if (element === undefined) {
    return;
  }
  // Enter and Space pressed on another focused link or button belong to it.
  const focused = enclosing(event.target, INTERACTIVE);
  if (ACTIVATION_KEYS.has(event.key) && focused !== null && focused !== element) {
    return;
  }
  // Preventing the default keeps a focused bound element from acting twice.
  event.preventDefault();
  element.click();
});

export {};
