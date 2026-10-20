# GeoData Rawfiles

Paws seeds meow-rs geodata from HarmonyOS rawfiles on app startup. Runtime
YAML points meow-rs at the app-private copies:

- `Country.mmdb`
- `GeoLite2-ASN.mmdb`
- `geosite.dat`

The rawfile directory is:

```sh
entry/src/main/resources/rawfile/geodata
```

Fetch or refresh the files with:

```sh
scripts/fetch-geodata.sh
```

The fetch script downloads all three files from MetaCubeX
`meta-rules-dat` releases.

The geodata binaries are ignored by git so routine source changes do not churn
large generated artifacts. CI and release runners should run the fetch script
or otherwise provision the three rawfiles before packaging release builds.
