# Sentences

The server reads git when a page opens. It parses every file that changed exactly once per visit. It caches the parsed blocks by blob id. A second visit within the hour reads the cache and nothing else. Old entries leave the cache after one day.
