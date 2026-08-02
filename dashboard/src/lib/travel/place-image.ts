import { wikimedia, type PlaceRef } from "$lib/api";

export interface PlaceImage {
  url: string;
  articleUrl: string;
  fileName: string;
}

interface WikiPage {
  fullurl?: unknown;
  pageimage?: unknown;
  thumbnail?: { source?: unknown };
}

export function placeName(place: PlaceRef): string {
  return place.name
    .split(",")[0]
    .replace(/\s+(Hauptbahnhof|Hbf|ZOB)$/i, "")
    .replace(/\s*\([^)]*\)\s*$/, "")
    .trim();
}

export async function loadPlaceImage(place: PlaceRef): Promise<PlaceImage | null> {
  const title = placeName(place);
  if (!title) return null;

  const body = await wikimedia.placeImage(title);
  if (!body || typeof body !== "object") return null;
  const query = (body as { query?: unknown }).query;
  if (!query || typeof query !== "object") return null;
  const pages = (query as { pages?: unknown }).pages;
  if (!pages || typeof pages !== "object") return null;

  const page = Object.values(pages)[0] as WikiPage | undefined;
  const url = page?.thumbnail?.source;
  const articleUrl = page?.fullurl;
  const fileName = page?.pageimage;
  if (
    typeof url !== "string" ||
    !url.startsWith("https://") ||
    typeof articleUrl !== "string" ||
    !articleUrl.startsWith("https://") ||
    typeof fileName !== "string"
  ) {
    return null;
  }
  return { url, articleUrl, fileName };
}
