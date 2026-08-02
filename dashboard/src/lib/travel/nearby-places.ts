import { wikimedia, type PlaceRef } from "$lib/api";

export interface NearbyPlace {
  id: string;
  title: string;
  description: string;
  url: string;
  imageUrl: string | null;
  latitude: number;
  longitude: number;
}

interface WikiNearbyPage {
  pageid?: unknown;
  title?: unknown;
  description?: unknown;
  fullurl?: unknown;
  thumbnail?: { source?: unknown };
  coordinates?: Array<{ lat?: unknown; lon?: unknown }>;
}

export async function loadNearbyPlaces(place: PlaceRef): Promise<NearbyPlace[]> {
  if (typeof place.latitude !== "number" || typeof place.longitude !== "number") return [];
  const body = await wikimedia.nearby(place.latitude, place.longitude);
  if (!body || typeof body !== "object") return [];
  const query = (body as { query?: unknown }).query;
  if (!query || typeof query !== "object") return [];
  const pages = (query as { pages?: unknown }).pages;
  if (!pages || typeof pages !== "object") return [];

  return Object.values(pages)
    .flatMap((raw): NearbyPlace[] => {
      const page = raw as WikiNearbyPage;
      const coordinate = page.coordinates?.[0];
      if (
        typeof page.pageid !== "number" ||
        typeof page.title !== "string" ||
        typeof page.fullurl !== "string" ||
        !page.fullurl.startsWith("https://") ||
        typeof coordinate?.lat !== "number" ||
        typeof coordinate.lon !== "number"
      ) {
        return [];
      }
      const imageUrl = page.thumbnail?.source;
      return [
        {
          id: `wikipedia:${page.pageid}`,
          title: page.title,
          description: typeof page.description === "string" ? page.description : "",
          url: page.fullurl,
          imageUrl:
            typeof imageUrl === "string" && imageUrl.startsWith("https://") ? imageUrl : null,
          latitude: coordinate.lat,
          longitude: coordinate.lon,
        },
      ];
    })
    .filter((candidate) => candidate.title !== place.name)
    .slice(0, 8);
}
