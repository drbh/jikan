import requests

# hacker News API endpoints
top_stories_url = "https://hacker-news.firebaseio.com/v0/topstories.json"
item_url = "https://hacker-news.firebaseio.com/v0/item/{}.json"


num_stories = 10

# fetch IDs of top stories
response = requests.get(top_stories_url)
if response.status_code != 200:
    print(f"Failed to retrieve top stories. HTTP Status Code: {response.status_code}")
    exit()

story_ids = response.json()[:num_stories]

# fetch details of each story
for id in story_ids:
    story_response = requests.get(item_url.format(id))
    if story_response.status_code == 200:
        story = story_response.json()
        print(f"Title: {story.get('title')}")
        print(f"URL: {story.get('url')}")
        print(f"Score: {story.get('score')}")
        print(f"By: {story.get('by')}")
        print("---")
    else:
        print(
            f"Failed to retrieve story {id}. HTTP Status Code: {story_response.status_code}"
        )
