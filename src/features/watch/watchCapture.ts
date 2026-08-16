export const WATCH_CAPTURE_MAX_BYTES = 4_500_000;
const SEGMENT_DURATION_MS = 12_000;
const MIN_USEFUL_CLIP_MS = 2_500;

export interface WatchCaptureMetadata {
  itemId: string;
  title: string;
  seriesName?: string;
  currentTime: number;
  duration: number;
  paused: boolean;
  subtitle?: string;
}

export interface WatchCaptureResult extends WatchCaptureMetadata {
  blob?: Blob;
  fileName?: string;
  mimeType?: string;
  captureKind: "clip" | "frame" | "metadata";
  clipStartTime?: number;
  clipEndTime?: number;
}

interface RecordedSegment {
  blob: Blob;
  startTime: number;
  endTime: number;
  durationMs: number;
}

type CapturableVideo = HTMLVideoElement & {
  captureStream?: () => MediaStream;
  mozCaptureStream?: () => MediaStream;
};

const chooseRecordingMime = () => {
  if (typeof MediaRecorder === "undefined") return "";
  return [
    "video/mp4;codecs=avc1.42E01E,mp4a.40.2",
    "video/webm;codecs=vp8,opus",
    "video/webm",
  ].find((mime) => MediaRecorder.isTypeSupported(mime)) || "";
};

const extensionForMime = (mime: string) =>
  mime.startsWith("video/mp4") ? "mp4" : "webm";

export class RollingWatchCapture {
  private readonly video: CapturableVideo;
  private readonly getMetadata: () => WatchCaptureMetadata;
  private stream: MediaStream | null = null;
  private recorder: MediaRecorder | null = null;
  private chunks: Blob[] = [];
  private segmentStartedAt = 0;
  private segmentStartedPosition = 0;
  private rotateTimer: number | null = null;
  private lastSegment: RecordedSegment | null = null;
  private queue: Promise<unknown> = Promise.resolve();
  private disposed = false;

  constructor(
    video: HTMLVideoElement,
    getMetadata: () => WatchCaptureMetadata,
  ) {
    this.video = video as CapturableVideo;
    this.getMetadata = getMetadata;
    this.video.addEventListener("play", this.handlePlay);
  }

  private handlePlay = () => {
    void this.enqueue(async () => this.startRecorder());
  };

  private enqueue<T>(operation: () => Promise<T>): Promise<T> {
    const next = this.queue.then(operation, operation);
    this.queue = next.then(
      () => undefined,
      () => undefined,
    );
    return next;
  }

  private ensureStream() {
    if (this.stream?.active) return this.stream;
    const capture = this.video.captureStream || this.video.mozCaptureStream;
    if (!capture) return null;
    const stream = capture.call(this.video);
    if (stream.getVideoTracks().length === 0) return null;
    this.stream = stream;
    return stream;
  }

  private async startRecorder() {
    if (this.disposed || this.recorder || this.video.paused) return;
    const stream = this.ensureStream();
    const mimeType = chooseRecordingMime();
    if (!stream || !mimeType) return;

    this.chunks = [];
    const recorder = new MediaRecorder(stream, {
      mimeType,
      videoBitsPerSecond: 2_100_000,
      audioBitsPerSecond: 64_000,
    });
    recorder.addEventListener("dataavailable", (event) => {
      if (event.data.size > 0) this.chunks.push(event.data);
    });
    recorder.addEventListener("error", () => {
      this.recorder = null;
      this.clearRotateTimer();
    });
    this.recorder = recorder;
    this.segmentStartedAt = Date.now();
    this.segmentStartedPosition = this.video.currentTime || 0;
    recorder.start();
    this.rotateTimer = window.setTimeout(() => {
      void this.enqueue(async () => {
        const completed = await this.stopRecorder();
        if (completed) this.lastSegment = completed;
        await this.startRecorder();
      });
    }, SEGMENT_DURATION_MS);
  }

  private clearRotateTimer() {
    if (this.rotateTimer !== null) {
      window.clearTimeout(this.rotateTimer);
      this.rotateTimer = null;
    }
  }

  private async stopRecorder(): Promise<RecordedSegment | null> {
    const recorder = this.recorder;
    if (!recorder) return null;
    this.clearRotateTimer();
    this.recorder = null;

    if (recorder.state !== "inactive") {
      const stopped = new Promise<void>((resolve) => {
        recorder.addEventListener("stop", () => resolve(), { once: true });
      });
      recorder.stop();
      await stopped;
    }

    const durationMs = Date.now() - this.segmentStartedAt;
    const blob = new Blob(this.chunks, { type: recorder.mimeType });
    this.chunks = [];
    if (!blob.size) return null;
    return {
      blob,
      startTime: this.segmentStartedPosition,
      endTime: this.video.currentTime || this.segmentStartedPosition,
      durationMs,
    };
  }

  private async captureFrame() {
    if (!this.video.videoWidth || !this.video.videoHeight) return null;
    const maxWidth = 1280;
    const scale = Math.min(1, maxWidth / this.video.videoWidth);
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(this.video.videoWidth * scale));
    canvas.height = Math.max(1, Math.round(this.video.videoHeight * scale));
    const context = canvas.getContext("2d");
    if (!context) return null;

    try {
      context.drawImage(this.video, 0, 0, canvas.width, canvas.height);
      return await new Promise<Blob | null>((resolve) =>
        canvas.toBlob(resolve, "image/jpeg", 0.82),
      );
    } catch {
      return null;
    }
  }

  async capture(): Promise<WatchCaptureResult> {
    return this.enqueue(async () => {
      const metadata = this.getMetadata();
      const currentSegment = await this.stopRecorder();
      const usableCurrent =
        currentSegment && currentSegment.durationMs >= MIN_USEFUL_CLIP_MS
          ? currentSegment
          : null;
      const clip = usableCurrent || this.lastSegment;

      if (usableCurrent) this.lastSegment = usableCurrent;
      await this.startRecorder();

      if (clip && clip.blob.size <= WATCH_CAPTURE_MAX_BYTES) {
        const mimeType = clip.blob.type || "video/webm";
        return {
          ...metadata,
          blob: clip.blob,
          mimeType,
          fileName: `watch-context-${Date.now()}.${extensionForMime(mimeType)}`,
          captureKind: "clip",
          clipStartTime: clip.startTime,
          clipEndTime: clip.endTime,
        };
      }

      const frame = await this.captureFrame();
      if (frame) {
        return {
          ...metadata,
          blob: frame,
          mimeType: "image/jpeg",
          fileName: `watch-context-${Date.now()}.jpg`,
          captureKind: "frame",
        };
      }

      return { ...metadata, captureKind: "metadata" };
    });
  }

  async start() {
    await this.enqueue(async () => this.startRecorder());
  }

  dispose() {
    this.disposed = true;
    this.video.removeEventListener("play", this.handlePlay);
    this.clearRotateTimer();
    if (this.recorder?.state !== "inactive") this.recorder?.stop();
    this.recorder = null;
    this.stream?.getTracks().forEach((track) => track.stop());
    this.stream = null;
  }
}
