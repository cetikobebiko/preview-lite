import { invokeProfiled as invokeContractProfiled } from './services/backend/client.js';

/**
 * Traced wrapper around Tauri's invoke command to measure IPC roundtrip time.
 * @param {string} cmd 
 * @param {object} [args] 
 * @returns {Promise<any>}
 */
export async function invokeProfiled(cmd, args) {
    return invokeContractProfiled(cmd, args);
}
