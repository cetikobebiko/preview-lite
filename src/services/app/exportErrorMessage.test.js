import { describe, expect, it } from "vitest";
import { BackendCommandError } from "../backend/client.js";
import { exportErrorMessage } from "./exportErrorMessage.js";

const jobError = (code, message) => Object.assign(new Error(message), { code });

describe("exportErrorMessage", () => {
  it("explains render limit failures and keeps the backend DPI suggestion", () => {
    const message = exportErrorMessage(
      jobError(
        "SESSION_JOB_RENDER_LIMIT_EXCEEDED",
        "Sayfa 1, 600 DPI'da 60000×60000 piksel olur; görsel dışa aktarım en fazla 16384×16384 piksel ve 256 MiB destekler. Bu sayfa için en fazla 81 DPI seçin.",
      ),
    );
    expect(message).toMatch(/^Seçilen çözünürlük bu sayfa için çok yüksek\. /);
    expect(message).toContain("en fazla 81 DPI");
    expect(message).not.toContain("Error:");
  });

  it("labels invalid DPI and page failures, including command-level errors", () => {
    expect(
      exportErrorMessage(
        jobError("SESSION_JOB_INVALID_EXPORT_DPI", "Çözünürlük 36–1200 DPI aralığında olmalıdır (istenen: -1)"),
      ),
    ).toBe("Geçersiz çözünürlük. Çözünürlük 36–1200 DPI aralığında olmalıdır (istenen: -1)");
    const commandError = new Error("Sayfa 9 belgede yok (sayfa sayısı: 3)");
    commandError.cause = { code: "SESSION_JOB_INVALID_PAGE" };
    expect(exportErrorMessage(commandError)).toBe(
      "Seçilen sayfa belgede bulunamadı. Sayfa 9 belgede yok (sayfa sayısı: 3)",
    );
  });

  it("explains a job worker stopped by the memory budget", () => {
    expect(
      exportErrorMessage(
        jobError("SESSION_JOB_WORKER_MEMORY_LIMIT", "Job worker bellek bütçesini aştığı için durduruldu (2100 MiB > 2048 MiB)"),
      ),
    ).toBe(
      "İşlem PDF motorunun bellek sınırını aştığı için durduruldu. Job worker bellek bütçesini aştığı için durduruldu (2100 MiB > 2048 MiB)",
    );
  });

  it("falls back to the raw message without an Error: prefix", () => {
    expect(exportErrorMessage(jobError("SESSION_JOB_WORKER_CRASHED", "Worker çöktü"))).toBe(
      "Worker çöktü",
    );
    expect(exportErrorMessage("düz metin hata")).toBe("düz metin hata");
    expect(exportErrorMessage(undefined)).toBe("Dışa aktarma başarısız oldu.");
    expect(exportErrorMessage(new Error(""))).toBe("Dışa aktarma başarısız oldu.");
  });

  it("keeps the worker-safety prefix of backend command errors", () => {
    const error = new BackendCommandError("exportSessionPagesAsImages", "Worker çöktü", {
      code: "PDFIUM_WORKER_CRASHED",
      message: "Worker çöktü",
    });
    expect(exportErrorMessage(error)).toBe(error.userMessage);
  });
});
