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

`country.mmdb` and `GeoLite2-ASN.mmdb` are downloaded from MetaCubeX
`meta-rules-dat`. meow-rs 0.17 auto-detects and loads the upstream V2Ray
`geosite.dat` format directly, so release builds do not require an additional
conversion CLI.

The geodata binaries are ignored by git so routine source changes do not churn
large generated artifacts. CI and release runners should run the fetch script
or otherwise provision the three rawfiles before packaging release builds.
