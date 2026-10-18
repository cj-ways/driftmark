# Sentences

The server reads git when a page opens. It parses each changed file once. It caches the parsed blocks by blob id. A second visit reads only the cache. Old entries leave the cache after one day.
