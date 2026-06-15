import { format, formatDistanceToNow, isToday, isYesterday } from "date-fns";
import { zhCN } from "date-fns/locale";
import type { VcpNotification } from "../../../core/stores/notification";

export function useNotificationPresentation() {
  const formatTime = (timestamp: number): string => {
    const date = new Date(timestamp);
    if (isToday(date)) {
      return format(date, "HH:mm:ss");
    }
    if (isYesterday(date)) {
      return `昨天 ${format(date, "HH:mm")}`;
    }
    return format(date, "yyyy-MM-dd HH:mm");
  };

  const formatRelativeTime = (timestamp: number): string => {
    try {
      return formatDistanceToNow(new Date(timestamp), {
        addSuffix: true,
        locale: zhCN,
      });
    } catch (e) {
      return "";
    }
  };

  const getDistance = (
    item: any,
  ): { value: string; isEstimated: boolean } | null => {
    if (!item) return null;

    const parseMetricNumber = (value: unknown): number | null => {
      if (typeof value === "number" && Number.isFinite(value)) return value;
      if (typeof value === "string" && value.trim()) {
        const parsed = Number.parseFloat(value);
        return Number.isFinite(parsed) ? parsed : null;
      }
      return null;
    };

    const distance = parseMetricNumber(item.distance);
    if (distance !== null) {
      return { value: distance.toFixed(4), isEstimated: false };
    }

    const score = parseMetricNumber(
      item.score ?? item.rerank_score ?? item.original_score,
    );
    if (score !== null && score > 0) {
      const est = 1 / score - 1;
      return { value: est.toFixed(4), isEstimated: true };
    }

    return null;
  };

  const getTypeColor = (type: VcpNotification["type"]) => {
    switch (type) {
      case "error":
        return {
          text: "text-rose-600",
          bg: "bg-rose-50",
          border: "border-rose-100",
          dot: "bg-rose-500",
        };
      case "warning":
        return {
          text: "text-amber-600",
          bg: "bg-amber-50",
          border: "border-amber-100",
          dot: "bg-amber-500",
        };
      case "success":
        return {
          text: "text-emerald-600",
          bg: "bg-emerald-50",
          border: "border-emerald-100",
          dot: "bg-emerald-500",
        };
      case "tool":
        return {
          text: "text-sky-600",
          bg: "bg-sky-50",
          border: "border-sky-100",
          dot: "bg-sky-400",
        };
      case "agent":
        return {
          text: "text-purple-600",
          bg: "bg-purple-50",
          border: "border-purple-100",
          dot: "bg-purple-400",
        };
      case "info":
      default:
        return {
          text: "text-slate-600",
          bg: "bg-slate-50",
          border: "border-slate-100",
          dot: "bg-slate-400",
        };
    }
  };

  const getActionButtonClass = (action: {
    color?: string;
    value?: boolean;
  }) => {
    if (action.color?.trim()) {
      return action.color;
    }
    return action.value
      ? "bg-green-500 text-white border-green-500 shadow-lg shadow-green-500/20"
      : "bg-red-500 text-white border-red-500 shadow-lg shadow-red-500/20";
  };

  const copyToClipboard = async (text: string): Promise<boolean> => {
    try {
      if (navigator.clipboard && navigator.clipboard.writeText) {
        await navigator.clipboard.writeText(text);
        return true;
      }
      const textArea = document.createElement("textarea");
      textArea.value = text;
      textArea.style.position = "fixed";
      textArea.style.opacity = "0";
      document.body.appendChild(textArea);
      try {
        textArea.focus();
        textArea.select();
        return document.execCommand("copy");
      } finally {
        document.body.removeChild(textArea);
      }
    } catch (err) {
      return false;
    }
  };

  return {
    formatTime,
    formatRelativeTime,
    getDistance,
    getTypeColor,
    getActionButtonClass,
    copyToClipboard,
  };
}
