import { authorFormFields } from "../pdfState.js";
import {
  ensureSignedFormAuthoringAccepted,
  isSignatureLossDeclined,
} from "../services/signatureLossGuard.js";

// Form tasarım modu durumu (docs/engineering/form-authoring-plan.md, Faz B).
//
// Sayfa katmanları (FormDesignLayer) ve araç çubuğu aynı durumu paylaşır: seçili widget,
// işlem sürüyor mu, son hata. Her işlem bir mutasyondur; başarıda yeni oturum anlık
// görüntüsü katmanın `onsessionSnapshot`'ına iletilir ve seçim, sayfa yeniden okunduktan
// sonra ada göre (ya da yeni eklenen widget için sayfanın son widget'ı olarak) yeniden kurulur.

export const FORM_DESIGN_CONTEXT = Symbol("form-design");

export const FORM_TOOLS = Object.freeze([
  "form-select",
  "form-text",
  "form-checkbox",
  "form-radio",
  "form-combo",
  "form-list",
]);

export function isFormTool(tool) {
  return FORM_TOOLS.includes(tool);
}

const defaultDependencies = Object.freeze({
  author: authorFormFields,
  ensureSigned: ensureSignedFormAuthoringAccepted,
});

export function createFormDesignState(dependencies = defaultDependencies) {
  // selection: { sessionId, contentGeneration, pageIndex, widget, onsessionSnapshot }
  // pendingSelect: { sessionId, pageIndex, fieldName } | { sessionId, pageIndex, newest: true }
  const state = $state({ selection: null, pendingSelect: null, busy: false, error: "" });

  async function run(context, ops, pendingSelect = null) {
    if (state.busy) return null;
    state.busy = true;
    state.error = "";
    try {
      await dependencies.ensureSigned(context.sessionId, context.contentGeneration, "Form alanı değişikliği");
      const snapshot = await dependencies.author(context.sessionId, context.contentGeneration, ops);
      state.selection = null;
      state.pendingSelect = pendingSelect ? { sessionId: context.sessionId, ...pendingSelect } : null;
      context.onsessionSnapshot?.(snapshot);
      return snapshot;
    } catch (error) {
      if (!isSignatureLossDeclined(error)) {
        state.error = error?.message || String(error || "Form alanı değiştirilemedi.");
        console.error("Form alanı değiştirilemedi", error);
      }
      return null;
    } finally {
      state.busy = false;
    }
  }

  function selectedOp(op, extra = {}) {
    const selection = state.selection;
    return {
      op,
      pageIndex: selection.pageIndex,
      annotIndex: selection.widget.annot_index,
      fieldName: selection.widget.field_name,
      ...extra,
    };
  }

  return {
    get selection() {
      return state.selection;
    },
    get pendingSelect() {
      return state.pendingSelect;
    },
    get busy() {
      return state.busy;
    },
    get error() {
      return state.error;
    },

    select(selection) {
      state.selection = selection;
      state.error = "";
    },

    clearSelection() {
      state.selection = null;
    },

    clearPendingSelect() {
      state.pendingSelect = null;
    },

    run,

    add(context, op) {
      return run(context, [op], { pageIndex: op.pageIndex, newest: true });
    },

    moveSelected(normalizedRect) {
      const selection = state.selection;
      if (!selection) return Promise.resolve(null);
      return run(selection, [selectedOp("move", { normalizedRect })], {
        pageIndex: selection.pageIndex,
        fieldName: selection.widget.field_name,
      });
    },

    updateSelected(props) {
      const selection = state.selection;
      if (!selection) return Promise.resolve(null);
      const fieldName = props.name ?? selection.widget.field_name;
      return run(selection, [selectedOp("set_props", { props })], {
        pageIndex: selection.pageIndex,
        fieldName,
      });
    },

    deleteSelected() {
      const selection = state.selection;
      if (!selection) return Promise.resolve(null);
      return run(selection, [selectedOp("delete")]);
    },
  };
}
