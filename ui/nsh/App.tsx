import { mount } from "nsh";
import { PawsApp } from "./src/app";
import { CoreClient } from "./src/client";

mount(() => {
  const client = new CoreClient();
  return <PawsApp client={client} />;
});
