const { QuickJS } = require("./index.js");

(async () => {
  const runtime = new QuickJS({});

  const result = await runtime.eval(`2+2`);
  console.log(result); // 4

  await runtime.close();
})()