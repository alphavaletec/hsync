// Logos das marcas por tipo de remote do rclone. Os desenhos vêm do Simple Icons (CC0),
// embutidos no app (nada é baixado). As marcas pertencem aos seus donos.
import {
  siBackblaze,
  siBox,
  siCitrix,
  siCloudinary,
  siDropbox,
  siFilen,
  siGooglecloud,
  siGoogledrive,
  siGooglephotos,
  siHuawei,
  siIcloud,
  siInternetarchive,
  siMaildotru,
  siMega,
  siOpenstack,
  siProtondrive,
  siSeafile,
  siZoho,
} from "simple-icons";

export interface ProviderLogo {
  title: string;
  hex: string;
  path: string;
}

// A Microsoft pediu a retirada dos seus logos do Simple Icons: para o OneDrive usamos
// uma nuvem no azul da marca.
const ONEDRIVE: ProviderLogo = {
  title: "OneDrive",
  hex: "0078D4",
  path: "M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z",
};

const LOGOS: Record<string, ProviderLogo> = {
  drive: siGoogledrive,
  onedrive: ONEDRIVE,
  mega: siMega,
  dropbox: siDropbox,
  box: siBox,
  b2: siBackblaze,
  protondrive: siProtondrive,
  seafile: siSeafile,
  internetarchive: siInternetarchive,
  iclouddrive: siIcloud,
  sharefile: siCitrix,
  zoho: siZoho,
  gcs: siGooglecloud,
  gphotos: siGooglephotos,
  filen: siFilen,
  mailru: siMaildotru,
  huaweidrive: siHuawei,
  cloudinary: siCloudinary,
  swift: siOpenstack,
};

export function providerLogo(kind: string): ProviderLogo | undefined {
  return LOGOS[kind];
}

/** Cor da marca legível no fundo escuro: marcas pretas ou muito escuras usam o texto padrão. */
export function readableColor(hex: string): string {
  const [r, g, b] = [0, 2, 4].map((i) => {
    const c = parseInt(hex.slice(i, i + 2), 16) / 255;
    return c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b < 0.05 ? "var(--color-fg)" : `#${hex}`;
}
