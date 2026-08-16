export interface EmbySession {
  accessToken: string;
  userId: string;
  userName: string;
  serverName?: string;
}

export interface EmbyMediaItem {
  Id: string;
  Name: string;
  Type: "Movie" | "Episode" | string;
  SeriesName?: string;
  ParentIndexNumber?: number;
  IndexNumber?: number;
  RunTimeTicks?: number;
  ProductionYear?: number;
  Overview?: string;
  ImageTags?: Record<string, string>;
  BackdropImageTags?: string[];
}

interface EmbyItemsResponse {
  Items?: EmbyMediaItem[];
  TotalRecordCount?: number;
}

export interface EmbyMediaStream {
  Index: number;
  Type: "Audio" | "Video" | "Subtitle" | string;
  Language?: string;
  DisplayTitle?: string;
  IsDefault?: boolean;
  IsExternal?: boolean;
  DeliveryUrl?: string;
}

export interface EmbyMediaSource {
  Id: string;
  Container?: string;
  RunTimeTicks?: number;
  MediaStreams?: EmbyMediaStream[];
}

export interface EmbyPlaybackInfo {
  PlaySessionId?: string;
  MediaSources?: EmbyMediaSource[];
}

export interface EmbyConnection {
  serverUrl: string;
  accessToken: string;
  userId: string;
  userName: string;
  deviceId: string;
  serverName?: string;
}

const CLIENT_NAME = "VCPMobile";
const CLIENT_VERSION = "1.1.4";

export const normalizeEmbyServerUrl = (value: string) =>
  value.trim().replace(/\/+$/, "");

const embyAuthorization = (deviceId: string, token?: string) => {
  const parts = [
    `Client="${CLIENT_NAME}"`,
    `Device="VCPMobile"`,
    `DeviceId="${deviceId}"`,
    `Version="${CLIENT_VERSION}"`,
  ];
  if (token) parts.push(`Token="${token}"`);
  return `MediaBrowser ${parts.join(", ")}`;
};

const parseError = async (response: Response) => {
  const text = await response.text().catch(() => "");
  return text.trim() || `${response.status} ${response.statusText}`;
};

export async function authenticateEmby(
  serverUrl: string,
  username: string,
  password: string,
  deviceId: string,
): Promise<EmbySession> {
  const baseUrl = normalizeEmbyServerUrl(serverUrl);
  const response = await fetch(`${baseUrl}/Users/AuthenticateByName`, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "X-Emby-Authorization": embyAuthorization(deviceId),
    },
    body: JSON.stringify({ Username: username.trim(), Pw: password }),
  });

  if (!response.ok) {
    throw new Error(`Emby 登录失败：${await parseError(response)}`);
  }

  const data = await response.json();
  if (!data?.AccessToken || !data?.User?.Id) {
    throw new Error("Emby 返回了无效的登录会话");
  }

  const publicInfo = await fetch(`${baseUrl}/System/Info/Public`)
    .then((result) => (result.ok ? result.json() : null))
    .catch(() => null);

  return {
    accessToken: data.AccessToken,
    userId: data.User.Id,
    userName: data.User.Name || username.trim(),
    serverName: publicInfo?.ServerName,
  };
}

export class EmbyClient {
  readonly connection: EmbyConnection;

  constructor(connection: EmbyConnection) {
    this.connection = {
      ...connection,
      serverUrl: normalizeEmbyServerUrl(connection.serverUrl),
    };
  }

  private async request<T>(
    path: string,
    init?: RequestInit,
  ): Promise<T> {
    const response = await fetch(`${this.connection.serverUrl}${path}`, {
      ...init,
      headers: {
        "Content-Type": "application/json",
        "X-Emby-Authorization": embyAuthorization(
          this.connection.deviceId,
          this.connection.accessToken,
        ),
        ...(init?.headers || {}),
      },
    });

    if (!response.ok) {
      throw new Error(await parseError(response));
    }
    if (response.status === 204) return undefined as T;
    return response.json() as Promise<T>;
  }

  async validateSession() {
    return this.request(`/Users/${encodeURIComponent(this.connection.userId)}`);
  }

  async getMediaItems(searchTerm = "", limit = 80) {
    const query = new URLSearchParams({
      UserId: this.connection.userId,
      Recursive: "true",
      IncludeItemTypes: "Movie,Episode",
      Fields:
        "Overview,PrimaryImageAspectRatio,DateCreated,ProductionYear,RunTimeTicks",
      SortBy: searchTerm ? "SortName" : "DateCreated,SortName",
      SortOrder: searchTerm ? "Ascending" : "Descending",
      Limit: String(limit),
      EnableImages: "true",
      ImageTypeLimit: "1",
    });
    if (searchTerm.trim()) query.set("SearchTerm", searchTerm.trim());

    const response = await this.request<EmbyItemsResponse>(
      `/Users/${encodeURIComponent(this.connection.userId)}/Items?${query}`,
    );
    return response.Items || [];
  }

  imageUrl(itemId: string, kind: "Primary" | "Backdrop" = "Primary") {
    const query = new URLSearchParams({
      api_key: this.connection.accessToken,
      maxWidth: kind === "Backdrop" ? "1600" : "500",
      quality: "88",
    });
    return `${this.connection.serverUrl}/Items/${encodeURIComponent(itemId)}/Images/${kind}?${query}`;
  }

  async getPlaybackInfo(itemId: string) {
    const query = new URLSearchParams({
      UserId: this.connection.userId,
    });
    return this.request<EmbyPlaybackInfo>(
      `/Items/${encodeURIComponent(itemId)}/PlaybackInfo?${query}`,
      {
        method: "POST",
        body: JSON.stringify({
          UserId: this.connection.userId,
          EnableDirectPlay: true,
          EnableDirectStream: true,
          EnableTranscoding: true,
        }),
      },
    );
  }

  hlsUrl(
    itemId: string,
    mediaSourceId: string,
    playSessionId?: string,
  ) {
    const query = new URLSearchParams({
      api_key: this.connection.accessToken,
      UserId: this.connection.userId,
      DeviceId: this.connection.deviceId,
      MediaSourceId: mediaSourceId,
      VideoCodec: "h264",
      AudioCodec: "aac",
      MaxStreamingBitrate: "12000000",
      TranscodingMaxAudioChannels: "2",
      EnableAutoStreamCopy: "true",
      AllowVideoStreamCopy: "true",
      AllowAudioStreamCopy: "true",
    });
    if (playSessionId) query.set("PlaySessionId", playSessionId);
    return `${this.connection.serverUrl}/Videos/${encodeURIComponent(itemId)}/master.m3u8?${query}`;
  }

  async reportPlayback(
    event: "Playing" | "Progress" | "Stopped",
    itemId: string,
    mediaSourceId: string,
    playSessionId: string | undefined,
    positionSeconds: number,
    isPaused: boolean,
  ) {
    const body = {
      ItemId: itemId,
      MediaSourceId: mediaSourceId,
      PlaySessionId: playSessionId,
      PositionTicks: Math.max(0, Math.floor(positionSeconds * 10_000_000)),
      IsPaused: isPaused,
      CanSeek: true,
    };
    await this.request(`/Sessions/Playing/${event}`, {
      method: "POST",
      body: JSON.stringify(body),
    }).catch(() => undefined);
  }
}
