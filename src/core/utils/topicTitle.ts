const DEFAULT_TOPIC_TITLE_PATTERN =
  /^(新话题|新会话)(?:\s+(?:\d{1,2}:\d{2}:\d{2}(?:\s?(?:AM|PM|am|pm|上午|下午))?|(?:AM|PM|am|pm|上午|下午)\s*\d{1,2}:\d{2}:\d{2}))?$/;

const padTimePart = (value: number) => String(value).padStart(2, "0");

export const createDefaultTopicTitle = (date: Date = new Date()) =>
  `新话题 ${padTimePart(date.getHours())}:${padTimePart(date.getMinutes())}:${padTimePart(date.getSeconds())}`;

export const isDefaultTopicTitle = (title?: string | null) =>
  Boolean(title && DEFAULT_TOPIC_TITLE_PATTERN.test(title.trim()));
