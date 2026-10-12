import { invoke } from "@tauri-apps/api/core";
import {
  getContract,
  getContractByCommand,
} from "./contracts.js";

const WORKER_SAFETY_FAILURE_CODES = new Set([
  "PDFIUM_WORKER_UNAVAILABLE",
  "PDFIUM_WORKER_SPAWN_FAILED",
  "PDFIUM_WORKER_INTERNAL",
  "PDFIUM_WORKER_LOAD_FAILED",
  "PDFIUM_WORKER_RENDER_FAILED",
  "PDFIUM_WORKER_CRASHED",
  "PDFIUM_WORKER_TIMEOUT",
  "PDFIUM_WORKER_PROTOCOL_MISMATCH",
  "PDFIUM_WORKER_PROTOCOL_ERROR",
  "PDFIUM_WORKER_MESSAGE_TOO_LARGE",
  "PDFIUM_WORKER_RESTART_LIMIT",
  "PDFIUM_WORKER_CLEANUP_FAILED",
  "PDFIUM_DOCUMENT_QUARANTINED",
  "PDFIUM_PAGE_QUARANTINED",
]);

export class BackendCommandError extends Error {
  constructor(command, message, cause) {
    super(message);
    this.name = "BackendCommandError";
    this.command = command;
    this.cause = cause;
    this.code =
      cause && typeof cause === "object" && typeof cause.code === "string"
        ? cause.code
        : null;
    this.userMessage = WORKER_SAFETY_FAILURE_CODES.has(this.code)
      ? `Belge güvenli şekilde işlenemedi ve işlem tamamlanmadı. ${message}`
      : message;
  }

  toString() {
    return `${this.name}: ${this.userMessage}`;
  }
}

function errorMessage(cause) {
  if (typeof cause === "string") return cause;
  if (cause instanceof Error && cause.message) return cause.message;
  if (
    cause &&
    typeof cause === "object" &&
    typeof cause.message === "string"
  ) {
    return cause.message;
  }
  return String(cause);
}

function normalizeError(command, cause) {
  if (cause instanceof BackendCommandError) return cause;
  return new BackendCommandError(command, errorMessage(cause), cause);
}

function validationError(path, message) {
  return new TypeError(`${path}: ${message}`);
}

function validateValue(value, schema, path) {
  if (value === null) {
    if (schema.nullable) return null;
    throw validationError(path, "null is not allowed");
  }

  if (schema.type === "string") {
    if (typeof value !== "string") {
      throw validationError(path, "expected string");
    }
    if (schema.minLength !== undefined && value.length < schema.minLength) {
      throw validationError(
        path,
        `expected string length >= ${schema.minLength}`,
      );
    }
    if (schema.maxLength !== undefined && value.length > schema.maxLength) {
      throw validationError(
        path,
        `expected string length <= ${schema.maxLength}`,
      );
    }
    if (schema.enum !== undefined && !schema.enum.includes(value)) {
      throw validationError(
        path,
        `expected one of: ${schema.enum.join(", ")}`,
      );
    }
    return value;
  }

  if (schema.type === "boolean") {
    if (typeof value !== "boolean") {
      throw validationError(path, "expected boolean");
    }
    return value;
  }

  if (schema.type === "number") {
    if (typeof value !== "number" || !Number.isFinite(value)) {
      throw validationError(path, "expected finite number");
    }
    if (schema.integer && !Number.isInteger(value)) {
      throw validationError(path, "expected integer");
    }
    if (schema.min !== undefined && value < schema.min) {
      throw validationError(path, `expected value >= ${schema.min}`);
    }
    if (schema.max !== undefined && value > schema.max) {
      throw validationError(path, `expected value <= ${schema.max}`);
    }
    return value;
  }

  if (schema.type === "array") {
    if (!Array.isArray(value)) {
      throw validationError(path, "expected array");
    }
    return value.map((item, index) =>
      validateValue(item, schema.items, `${path}[${index}]`),
    );
  }

  if (schema.type === "tuple") {
    if (!Array.isArray(value) || value.length !== schema.items.length) {
      throw validationError(
        path,
        `expected tuple of length ${schema.items.length}`,
      );
    }
    return schema.items.map((itemSchema, index) =>
      validateValue(value[index], itemSchema, `${path}[${index}]`),
    );
  }

  if (schema.type === "object") {
    if (!value || typeof value !== "object" || Array.isArray(value)) {
      throw validationError(path, "expected object");
    }

    const required = schema.required ?? {};
    const optional = schema.optional ?? {};
    const allowedKeys = new Set([
      ...Object.keys(required),
      ...Object.keys(optional),
    ]);

    for (const key of Object.keys(value)) {
      if (!allowedKeys.has(key)) {
        throw validationError(`${path}.${key}`, "unknown field");
      }
    }

    const validated = {};
    for (const [key, fieldSchema] of Object.entries(required)) {
      if (
        !Object.prototype.hasOwnProperty.call(value, key) ||
        value[key] === undefined
      ) {
        throw validationError(`${path}.${key}`, "required field is missing");
      }
      validated[key] = validateValue(
        value[key],
        fieldSchema,
        `${path}.${key}`,
      );
    }

    for (const [key, fieldSchema] of Object.entries(optional)) {
      if (
        !Object.prototype.hasOwnProperty.call(value, key) ||
        value[key] === undefined
      ) {
        continue;
      }
      validated[key] = validateValue(
        value[key],
        fieldSchema,
        `${path}.${key}`,
      );
    }

    if (typeof schema.validate === "function") {
      const message = schema.validate(validated);
      if (message) {
        throw validationError(path, message);
      }
    }

    return validated;
  }

  throw validationError(path, `unsupported schema type: ${schema.type}`);
}

function resolveContract(contractId) {
  const contract = getContract(contractId);
  if (!contract) {
    throw new BackendCommandError(
      String(contractId),
      `Unknown backend contract: ${String(contractId)}`,
      new TypeError(`Unknown backend contract: ${String(contractId)}`),
    );
  }
  return contract;
}

export async function callBackend(contractId, payload) {
  const contract = resolveContract(contractId);
  try {
    if (contract.payload === null) {
      if (payload !== undefined) {
        throw validationError(contract.command, "command accepts no payload");
      }
      const result = await invoke(contract.command);
      return contract.response ? contract.response.decode(result) : result;
    }

    const validatedPayload = validateValue(
      payload,
      contract.payload,
      contract.command,
    );
    const result = await invoke(contract.command, validatedPayload);
    return contract.response ? contract.response.decode(result) : result;
  } catch (cause) {
    throw normalizeError(contract.command, cause);
  }
}

export async function callBackendProfiled(contractId, payload) {
  const contract = resolveContract(contractId);
  const startedAt = performance.now();
  try {
    const result = await callBackend(contractId, payload);
    const payloadBytes =
      contract.response?.type === "binary" && result?.data?.buffer
        ? ` (${result.data.buffer.byteLength} bytes)`
        : "";
    console.log(
      `[IPC Profiler] ${contract.command} took ${(performance.now() - startedAt).toFixed(2)}ms${payloadBytes}`,
    );
    return result;
  } catch (error) {
    console.error(
      `[IPC Profiler] ${contract.command} failed after ${(performance.now() - startedAt).toFixed(2)}ms`,
    );
    throw error;
  }
}

export async function invokeProfiled(commandName, payload) {
  const match = getContractByCommand(commandName);
  if (!match) {
    throw new BackendCommandError(
      String(commandName),
      `Unknown backend command: ${String(commandName)}`,
      new TypeError(`Unknown backend command: ${String(commandName)}`),
    );
  }

  const [contractId] = match;
  return callBackendProfiled(contractId, payload);
}
