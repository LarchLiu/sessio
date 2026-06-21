/* eslint-disable */
// @ts-nocheck

Promise.withResolvers ??= function withResolvers() {
  var resolveFn,
    rejectFn,
    promise = new this(function (resolve, reject) {
      resolveFn = resolve;
      rejectFn = reject;
    });
  return { resolve: resolveFn, reject: rejectFn, promise };
};
