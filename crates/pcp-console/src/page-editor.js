import { icon } from "./ui-icons.js";

// The captured revision is the save precondition. Failed saves retain the draft.
export class PageEditSession {
  constructor({ request, mutate }) {
    this.request = request;
    this.mutate = mutate;
    this.snapshot = null;
    this.content = "";
    this.busy = false;
  }
  get dirty() { return this.snapshot !== null && this.content !== this.snapshot.content; }
  async open(pageId) {
    if (this.busy) return;
    this.busy = true;
    try {
      const snapshot = await this.request(`/api/pages/${encodeURIComponent(pageId)}/edit`);
      this.snapshot = snapshot;
      this.content = snapshot.content;
    } finally { this.busy = false; }
  }
  async save() {
    if (this.busy || !this.dirty) return null;
    this.busy = true;
    const content = this.content;
    try {
      const result = await this.mutate(`/api/pages/${encodeURIComponent(this.snapshot.pageId)}/edit`, {
        expectedRevisionId: this.snapshot.revisionId, content,
      });
      this.snapshot = { ...this.snapshot, content, revisionId: result.revisionId };
      return result;
    } finally { this.busy = false; }
  }
  reset() { this.snapshot = null; this.content = ""; }
}

export function createPageEditor({ request, mutate, confirmAction, t, onSaved, onDeleted }) {
  const byId = (id) => document.getElementById(id);
  const dialog = byId("page-dialog");
  const pane = byId("page-editor");
  const textarea = byId("page-editor-content");
  const notice = byId("page-action-notice");
  const edit = byId("page-edit");
  const remove = byId("page-delete");
  const save = byId("page-save");
  const cancel = byId("page-edit-cancel");
  const session = new PageEditSession({ request, mutate });
  let page = null;
  let editing = false;
  let deleting = false;
  let confirming = false;

  [[edit, "edit"], [remove, "delete"], [save, "accept"], [cancel, "undo"]].forEach(([button, name]) => button.append(icon(name)));

  function status(message = "", error = false) {
    notice.textContent = message;
    notice.hidden = !message;
    notice.classList.toggle("page-action-error", error);
    notice.setAttribute("role", error ? "alert" : "status");
  }
  function render() {
    const busy = session.busy || deleting || confirming;
    dialog.classList.toggle("is-editing", editing);
    dialog.setAttribute("aria-busy", String(busy));
    pane.hidden = !editing;
    edit.hidden = editing || !page?.actions?.canEdit;
    remove.hidden = editing || !page?.actions?.canDelete;
    save.hidden = cancel.hidden = !editing;
    edit.disabled = remove.disabled = save.disabled = cancel.disabled = busy;
    save.disabled ||= !session.dirty;
    textarea.readOnly = busy;
    byId("dialog-close").disabled = byId("dialog-back").disabled = busy;
    save.classList.toggle("is-loading", session.busy);
    remove.classList.toggle("is-loading", deleting);
  }
  function showError(error) {
    status(/revision conflict/i.test(error.message)
      ? t("This Page changed. Your draft is kept; copy it before reopening the editor.")
      : error.message, true);
  }
  async function begin() {
    if (!page || session.busy || deleting) return;
    editing = true;
    textarea.value = "";
    status(t("Loading content"));
    const loading = session.open(page.page.pageId);
    render();
    try {
      await loading;
      textarea.value = session.content;
      status();
      textarea.focus();
    } catch (error) { showError(error); }
    finally { render(); }
  }
  async function finish() {
    if (!session.dirty || session.busy) return;
    status(t("Saving changes…"));
    const saving = session.save();
    render();
    try {
      const result = await saving;
      if (!result) return;
      editing = false;
      session.reset();
      render();
      await onSaved(page.page.pageId);
      status(t("Saved"));
    } catch (error) { showError(error); }
    finally { render(); }
  }
  async function leave() {
    if (session.busy || deleting || confirming) return false;
    if (session.dirty) {
      confirming = true;
      render();
      let discard;
      try { discard = await confirmAction({title:t("Discard unsaved changes?"), description:t("Your saved Page will not change."), confirmLabel:t("Discard changes")}); }
      finally { confirming = false; render(); }
      if (!discard) return false;
    }
    editing = false;
    session.reset();
    status();
    render();
    return true;
  }
  async function deletePage() {
    if (!page || deleting || session.busy || confirming) return;
    const target = page;
    confirming = true;
    render();
    let confirmed;
    try {
      confirmed = await confirmAction({
        title:t("Delete this Page?"),
        description:`${target.revision.facets?.title || target.page.pageId}\n\n${t("This Page will leave retrieval immediately. Other Pages are not deleted; history is retained.")}`,
        confirmLabel:t("Delete Page"),
      });
    } finally { confirming = false; render(); }
    if (!confirmed) return;
    deleting = true;
    status(t("Deleting Page…"));
    render();
    try {
      await mutate(`/api/pages/${encodeURIComponent(target.page.pageId)}/delete`, {expectedRevisionId:target.revision.revisionId});
      page = null;
      dialog.close();
      await onDeleted(target.page.pageId);
      status();
    } catch (error) { showError(error); }
    finally { deleting = false; render(); }
  }
  textarea.addEventListener("input", () => { session.content = textarea.value; render(); });
  textarea.addEventListener("keydown", (event) => {
    if ((event.metaKey || event.ctrlKey) && event.key === "Enter") { event.preventDefault(); void finish(); }
  });
  edit.addEventListener("click", begin);
  remove.addEventListener("click", deletePage);
  save.addEventListener("click", finish);
  cancel.addEventListener("click", leave);
  return {
    leave, begin,
    attach(value) { page = value; editing = false; session.reset(); status(); render(); },
  };
}
